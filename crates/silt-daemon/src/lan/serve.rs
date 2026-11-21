//! The network listener's sockets (ADR 0112): HTTPS for paired phones,
//! and plain HTTP that serves only the authority's certificate. Handshakes
//! run each in a task of their own, so a slow one holds up no other.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use super::certs::Current;

/// How long a phone has to finish its TLS handshake.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// Handshakes done, waiting for the server to take them.
const READY: usize = 64;

/// Listens on every interface, IPv4, at `port`.
pub(crate) async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await
}

/// Where a phone's connection comes from, for the person deciding on it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Peer(pub(crate) SocketAddr);

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, Handshaken>> for Peer {
    fn connect_info(stream: axum::serve::IncomingStream<'_, Handshaken>) -> Self {
        Self(*stream.remote_addr())
    }
}

/// Connections whose handshake is done, for axum to serve.
pub(crate) struct Handshaken {
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
}

impl axum::serve::Listener for Handshaken {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(ready) => ready,
            // The accepting task stopped: so does the server, with it.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// Serves `app` over TLS with the current certificate.
pub(crate) fn secure(listener: TcpListener, current: Arc<Current>, app: Router) -> tokio::task::AbortHandle {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map(|builder| builder.with_no_client_auth().with_cert_resolver(current));
    let mut config = match config {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(%error, "could not set up TLS for phones on the network");
            return tokio::spawn(async {}).abort_handle();
        }
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let local = listener.local_addr().unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    let (sender, ready) = mpsc::channel(READY);
    tokio::spawn(async move {
        let accepting = async {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    // Out of files, say: let some close rather than spin.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                };
                let (acceptor, sender) = (acceptor.clone(), sender.clone());
                tokio::spawn(async move {
                    if let Ok(Ok(tls)) = tokio::time::timeout(HANDSHAKE, acceptor.accept(stream)).await {
                        let _ = sender.send((tls, peer)).await;
                    }
                });
            }
        };
        let serving = axum::serve(Handshaken { ready, local }, app.into_make_service_with_connect_info::<Peer>());
        tokio::select! {
            () = accepting => {}
            served = serving.into_future() => {
                if let Err(error) = served {
                    tracing::warn!(%error, "the network listener stopped");
                }
            }
        }
    })
    .abort_handle()
}

/// Serves `app` over plain HTTP.
pub(crate) fn plain(listener: TcpListener, app: Router) -> tokio::task::AbortHandle {
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::warn!(%error, "the certificate's page stopped");
        }
    })
    .abort_handle()
}
