//! The gateway as a proxy: it adds a credential only where its rules say,
//! passes the rest through unread, and refuses what sandboxes never reach.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use silt_gateway::{Ca, Credential, Gateway, Rules};
use silt_workspace::Network;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

/// A TLS server for `localhost` that answers each request with the
/// `authorization` header it got.
async fn echo_server() -> (u16, rustls::pki_types::CertificateDer<'static>) {
    let key = rcgen::KeyPair::generate().unwrap();
    let certificate = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap().self_signed(&key).unwrap();
    let der = certificate.der().clone();
    let private = rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into());
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], private)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else { return };
                let service = service_fn(|request: hyper::Request<hyper::body::Incoming>| async move {
                    let said = request
                        .headers()
                        .get("authorization")
                        .map(|value| value.to_str().unwrap_or("").to_owned())
                        .unwrap_or_default();
                    Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(Bytes::from(said))))
                });
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(tls), service).await;
            });
        }
    });
    (port, der)
}

/// A gateway answering on the returned port.
async fn gateway(rules: Rules, root: rustls::pki_types::CertificateDer<'static>) -> (tempfile::TempDir, Arc<Ca>, u16) {
    let dir = tempfile::tempdir().unwrap();
    let ca = Arc::new(Ca::load_or_create(&dir.path().join("ca")).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let gateway = Arc::new(Gateway::trusting(rules, ca.clone(), root));
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(gateway.clone().answer(stream));
        }
    });
    (dir, ca, port)
}

/// Sends `head` and returns the answer's first line and what followed it.
async fn ask(stream: &mut TcpStream, head: &str) -> String {
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut answer = vec![0u8; 4096];
    let read = stream.read(&mut answer).await.unwrap();
    String::from_utf8_lossy(&answer[..read]).into_owned()
}

/// A request through a CONNECT tunnel, trusting the gateway's authority.
async fn through_tunnel(gateway: u16, ca: &Ca, port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", gateway)).await.unwrap();
    let said = ask(&mut stream, &format!("CONNECT localhost:{port} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n")).await;
    assert!(said.starts_with("HTTP/1.1 200"), "{said}");
    let mut roots = rustls::RootCertStore::empty();
    roots.add(rustls::pki_types::pem::PemObject::from_pem_slice(ca.pem().as_bytes()).unwrap()).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .expect("the gateway's certificate is trusted");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
    tokio::spawn(connection);
    let request = hyper::Request::get(path)
        .header("host", format!("localhost:{port}"))
        .header("authorization", "Bearer silt-placeholder")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    String::from_utf8_lossy(&response.into_body().collect().await.unwrap().to_bytes()).into_owned()
}

#[tokio::test]
async fn a_credential_is_added_on_the_way_out_and_only_where_it_belongs() {
    let (port, root) = echo_server().await;
    let rules = Rules {
        network: Network::Full,
        hosts: Vec::new(),
        blocked: Vec::new(),
        credentials: vec![Credential {
            host: "localhost".into(),
            path: "/repos/acme/".into(),
            header: "authorization".into(),
            value: "Bearer s3cret".into(),
        }],
    };
    let (_dir, ca, at) = gateway(rules, root).await;
    assert_eq!(through_tunnel(at, &ca, port, "/repos/acme/api").await, "Bearer s3cret");
    assert_eq!(through_tunnel(at, &ca, port, "/repos/other/x").await, "Bearer silt-placeholder", "not its path");
}

#[tokio::test]
async fn what_sandboxes_never_reach_is_refused_with_the_reason() {
    let (_, root) = echo_server().await;
    let rules = Rules {
        network: Network::Gateway,
        hosts: vec!["*.github.com".into()],
        blocked: silt_workspace::subtract(&silt_workspace::ALWAYS_BLOCKED.map(str::to_owned), &[]),
        credentials: Vec::new(),
    };
    let (_dir, _ca, at) = gateway(rules, root).await;
    let mut stream = TcpStream::connect(("127.0.0.1", at)).await.unwrap();
    let said = ask(&mut stream, "CONNECT evil.example:443 HTTP/1.1\r\nHost: evil.example:443\r\n\r\n").await;
    assert!(said.starts_with("HTTP/1.1 403") && said.contains("isn't among the hosts"), "{said}");
    let mut stream = TcpStream::connect(("127.0.0.1", at)).await.unwrap();
    let said = ask(&mut stream, "GET http://169.254.169.254/latest/meta-data HTTP/1.1\r\nHost: 169.254.169.254\r\n\r\n").await;
    assert!(said.starts_with("HTTP/1.1 403"), "{said}");
}
