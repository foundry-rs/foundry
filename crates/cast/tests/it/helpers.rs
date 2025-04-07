use alloy_provider::{network::AnyNetwork, ProviderBuilder};
use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tokio::sync::oneshot;

pub async fn spawn_http_server() -> (alloy_provider::RootProvider<AnyNetwork>, ServerHandle) {
    let (tx, rx) = oneshot::channel();
    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/", post(|| async { "Hello, World!" }));

    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let server = axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            rx.await.ok();
        });

    let addr = server.local_addr();
    tokio::spawn(server);

    let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
        .on_builtin(format!("http://{}", addr))
        .await
        .unwrap();

    (provider, ServerHandle { shutdown: Some(tx) })
}

pub struct ServerHandle {
    shutdown: Option<oneshot::Sender<()>>,
}

impl ServerHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
} 