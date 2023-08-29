use futures_util::{SinkExt, StreamExt};
use mdbook::{utils::fs::get_404_output_file, MDBook};
use std::{
    net::{SocketAddr, ToSocketAddrs},
    path::PathBuf,
};
use tokio::sync::broadcast;
use warp::{ws::Message, Filter};

/// The HTTP endpoint for the websocket used to trigger reloads when a file changes.
const LIVE_RELOAD_ENDPOINT: &str = "__livereload";

/// Basic mdbook server. Given a path, hostname and port, serves the mdbook.
#[derive(Debug)]
pub struct Server {
    path: PathBuf,
    hostname: String,
    port: usize,
}

impl Default for Server {
    fn default() -> Self {
        Self { path: PathBuf::default(), hostname: "localhost".to_owned(), port: 3000 }
    }
}

impl Server {
    /// Create new instance of [Server].
    pub fn new(path: PathBuf) -> Self {
        Self { path, ..Default::default() }
    }

    /// Set host on the [Server].
    pub fn with_hostname(mut self, hostname: String) -> Self {
        self.hostname = hostname;
        self
    }

    /// Set port on the [Server].
    pub fn with_port(mut self, port: usize) -> Self {
        self.port = port;
        self
    }

    /// Serve the mdbook.
    pub fn serve(self) -> eyre::Result<()> {
        let mut book =
            MDBook::load(&self.path).map_err(|err| eyre::eyre!("failed to load book: {err:?}"))?;

        let address = format!("{}:{}", self.hostname, self.port);

        let update_config = |book: &mut MDBook| {
            book.config
                .set("output.html.live-reload-endpoint", LIVE_RELOAD_ENDPOINT)
                .expect("live-reload-endpoint update failed");
            // Override site-url for local serving of the 404 file
            book.config.set("output.html.site-url", "/").unwrap();
        };
        update_config(&mut book);
        book.build().map_err(|err| eyre::eyre!("failed to build book: {err:?}"))?;

        let sockaddr: SocketAddr = address
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| eyre::eyre!("no address found for {}", address))?;
        let build_dir = book.build_dir_for("html");
        let input_404 = book
            .config
            .get("output.html.input-404")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let file_404 = get_404_output_file(&input_404);

        // A channel used to broadcast to any websockets to reload when a file changes.
        let (tx, _rx) = tokio::sync::broadcast::channel::<Message>(100);

        sh_println!("Serving on: http://{address}")?;
        serve(build_dir, sockaddr, tx, &file_404);
        Ok(())
    }
}

// Adapted from https://github.com/rust-lang/mdBook/blob/41a6f0d43e1a2d9543877eacb4cd2a017f9fe8da/src/cmd/serve.rs#L124
#[tokio::main]
async fn serve(
    build_dir: PathBuf,
    address: SocketAddr,
    reload_tx: broadcast::Sender<Message>,
    file_404: &str,
) {
    // A warp Filter which captures `reload_tx` and provides an `rx` copy to
    // receive reload messages.
    let sender = warp::any().map(move || reload_tx.subscribe());

    // A warp Filter to handle the livereload endpoint. This upgrades to a
    // websocket, and then waits for any filesystem change notifications, and
    // relays them over the websocket.
    let livereload = warp::path(LIVE_RELOAD_ENDPOINT).and(warp::ws()).and(sender).map(
        |ws: warp::ws::Ws, mut rx: broadcast::Receiver<Message>| {
            ws.on_upgrade(move |ws| async move {
                let (mut user_ws_tx, _user_ws_rx) = ws.split();
                if let Ok(m) = rx.recv().await {
                    let _ = user_ws_tx.send(m).await;
                }
            })
        },
    );
    // A warp Filter that serves from the filesystem.
    let book_route = warp::fs::dir(build_dir.clone());
    // The fallback route for 404 errors
    let fallback_route = warp::fs::file(build_dir.join(file_404))
        .map(|reply| warp::reply::with_status(reply, warp::http::StatusCode::NOT_FOUND));
    let routes = livereload.or(book_route).or(fallback_route);
    warp::serve(routes).run(address).await;
}
