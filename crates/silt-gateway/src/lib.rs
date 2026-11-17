//! The gateway a sandboxed agent's traffic leaves through (ADR 0093). It
//! answers as an HTTP proxy on a socket the sandbox reaches: it refuses what
//! the run's rules don't allow, passes the rest through unread, and reads
//! only requests to hosts it holds a credential for, to add it on the way
//! out. The credential never enters the sandbox.

mod ca;
mod rules;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpStream, UnixListener};

pub use ca::Ca;
pub use rules::{Credential, Rules, matches};

type Body = BoxBody<Bytes, hyper::Error>;

/// One run's gateway.
pub struct Gateway {
    rules: Rules,
    ca: Arc<Ca>,
    upstream: Arc<rustls::ClientConfig>,
    /// Where it notes what it refused and the credentials it added.
    log: Option<PathBuf>,
}

impl Gateway {
    pub fn new(rules: Rules, ca: Arc<Ca>, log: Option<PathBuf>) -> Self {
        Self { rules, ca, upstream: Arc::new(upstream_config(None)), log }
    }

    /// The same, trusting only `root` upstream, for tests.
    pub fn trusting(rules: Rules, ca: Arc<Ca>, root: rustls::pki_types::CertificateDer<'static>) -> Self {
        Self { rules, ca, upstream: Arc::new(upstream_config(Some(root))), log: None }
    }

    /// Answers connections on `listener` until it's dropped.
    pub async fn serve(self: Arc<Self>, listener: UnixListener) {
        while let Ok((stream, _)) = listener.accept().await {
            let gateway = self.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let gateway = gateway.clone();
                    async move { Ok::<_, Infallible>(gateway.route(request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .preserve_header_case(true)
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    }

    async fn route(self: Arc<Self>, request: Request<Incoming>) -> Response<Body> {
        let Some(authority) = request.uri().authority().cloned() else {
            return refuse(StatusCode::BAD_REQUEST, "the gateway only takes requests for a host, as a proxy does");
        };
        let host = authority.host().trim_start_matches('[').trim_end_matches(']').to_owned();
        let tls = request.method() == Method::CONNECT;
        let port = authority.port_u16().unwrap_or(if tls { 443 } else { 80 });
        let address = match self.check(&host, port).await {
            Ok(address) => address,
            Err(reason) => {
                self.note(&host, port, Some(&reason), false);
                return refuse(StatusCode::FORBIDDEN, &format!("Silt's gateway refused {host}: {reason}"));
            }
        };
        if !tls {
            return self.forward(request, &host, port, address, false).await;
        }
        let gateway = self.clone();
        tokio::spawn(async move {
            let Ok(upgraded) = hyper::upgrade::on(request).await else { return };
            let client = TokioIo::new(upgraded);
            if gateway.rules.terminates(&host) {
                gateway.read_through(client, host, port, address).await;
            } else if let Ok(mut server) = TcpStream::connect(address).await {
                let mut client = client;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                let _ = server.shutdown().await;
            }
        });
        Response::new(empty())
    }

    /// Where `host` may be reached, or why not.
    async fn check(&self, host: &str, port: u16) -> Result<SocketAddr, String> {
        self.rules.host_allowed(host)?;
        let found = tokio::net::lookup_host((host, port)).await.map_err(|error| format!("{host} couldn't be found: {error}"))?;
        let mut first = None;
        for address in found {
            if !self.rules.address_allowed(address.ip()) {
                return Err(format!(
                    "{host} is {}, which sandboxes never reach: this machine, its networks and cloud metadata stay out",
                    address.ip()
                ));
            }
            first.get_or_insert(address);
        }
        first.ok_or_else(|| format!("{host} has no address"))
    }

    /// Reads the client's requests over TLS, as `host`, to add credentials.
    async fn read_through(
        self: Arc<Self>,
        client: TokioIo<hyper::upgrade::Upgraded>,
        host: String,
        port: u16,
        address: SocketAddr,
    ) {
        let Ok(leaf) = self.ca.leaf(&host) else { return };
        let mut config = match rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
        {
            Ok(builder) => builder.with_no_client_auth().with_cert_resolver(Arc::new(Fixed(leaf))),
            Err(_) => return,
        };
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let Ok(tls) = tokio_rustls::TlsAcceptor::from(Arc::new(config)).accept(client).await else { return };
        let gateway = self.clone();
        let service = service_fn(move |request| {
            let gateway = gateway.clone();
            let host = host.clone();
            async move { Ok::<_, Infallible>(gateway.forward(request, &host, port, address, true).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(tls), service).await;
    }

    /// Sends `request` on to `host`, with the credentials its rules add.
    async fn forward(
        &self,
        mut request: Request<Incoming>,
        host: &str,
        port: u16,
        address: SocketAddr,
        tls: bool,
    ) -> Response<Body> {
        let path = request.uri().path().to_owned();
        let headers = request.headers_mut();
        headers.remove("proxy-authorization");
        headers.remove("proxy-connection");
        let mut added = false;
        for credential in self.rules.credentials_for(host, &path) {
            if let (Ok(name), Ok(value)) =
                (HeaderName::from_bytes(credential.header.as_bytes()), HeaderValue::from_str(&credential.value))
            {
                headers.insert(name, value);
                added = true;
            }
        }
        if !headers.contains_key(hyper::header::HOST)
            && let Ok(value) =
                HeaderValue::from_str(&if port == 443 || port == 80 { host.to_owned() } else { format!("{host}:{port}") })
        {
            headers.insert(hyper::header::HOST, value);
        }
        // Origin-form, as a server expects it.
        if let Some(target) = request.uri().path_and_query().cloned()
            && let Ok(uri) = target.as_str().parse()
        {
            *request.uri_mut() = uri;
        }
        self.note(host, port, None, added);
        match self.send(request, host, address, tls).await {
            Ok(response) => response.map(|body| body.boxed()),
            Err(error) => refuse(StatusCode::BAD_GATEWAY, &format!("Silt's gateway couldn't reach {host}: {error}")),
        }
    }

    async fn send(
        &self,
        request: Request<Incoming>,
        host: &str,
        address: SocketAddr,
        tls: bool,
    ) -> Result<Response<Incoming>, String> {
        let tcp = TcpStream::connect(address).await.map_err(|error| error.to_string())?;
        if tls {
            let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|error| error.to_string())?;
            let stream = tokio_rustls::TlsConnector::from(self.upstream.clone())
                .connect(name, tcp)
                .await
                .map_err(|error| error.to_string())?;
            send_on(TokioIo::new(stream), request).await
        } else {
            send_on(TokioIo::new(tcp), request).await
        }
    }

    fn note(&self, host: &str, port: u16, refused: Option<&str>, credential: bool) {
        let Some(log) = &self.log else { return };
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis());
        let line =
            serde_json::json!({ "time_ms": time, "host": host, "port": port, "refused": refused, "credential": credential });
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = std::io::Write::write_all(&mut file, format!("{line}\n").as_bytes());
        }
    }
}

/// Passes each connection on `listener` to `port` on this machine: a port
/// a workspace hands its sandboxes, such as a dev database's.
pub async fn forward_port(listener: UnixListener, port: u16) {
    while let Ok((mut client, _)) = listener.accept().await {
        tokio::spawn(async move {
            if let Ok(mut server) = TcpStream::connect(("127.0.0.1", port)).await {
                let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
            }
        });
    }
}

async fn send_on<T>(io: TokioIo<T>, request: Request<Incoming>) -> Result<Response<Incoming>, String>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io).await.map_err(|error| error.to_string())?;
    tokio::spawn(connection);
    sender.send_request(request).await.map_err(|error| error.to_string())
}

/// The certificate for the one host a connection asked for.
#[derive(Debug)]
struct Fixed(Arc<rustls::sign::CertifiedKey>);

impl rustls::server::ResolvesServerCert for Fixed {
    fn resolve(&self, _hello: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// What the gateway trusts upstream: the system's roots and the public
/// ones; or only `root`, for tests.
fn upstream_config(root: Option<rustls::pki_types::CertificateDer<'static>>) -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    match root {
        Some(root) => {
            let _ = roots.add(root);
        }
        // The system's, which a company's own roots may be among, and the
        // public ones besides, where `SSL_CERT_FILE` names a store without them.
        None => {
            for certificate in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(certificate);
            }
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
    }
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap_or_else(|_| {
            rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .expect("TLS 1.3")
        });
    let mut config = builder.with_root_certificates(roots).with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

fn empty() -> Body {
    Full::new(Bytes::new()).map_err(|never| match never {}).boxed()
}

fn refuse(status: StatusCode, said: &str) -> Response<Body> {
    let mut response = Response::new(Full::new(Bytes::from(format!("{said}\n"))).map_err(|never| match never {}).boxed());
    *response.status_mut() = status;
    response.headers_mut().insert("x-silt-gateway", HeaderValue::from_static("refused"));
    response
}
