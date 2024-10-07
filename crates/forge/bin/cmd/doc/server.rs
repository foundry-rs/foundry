use axum::{routing::get_service, Router};
use forge_doc::mdbook::{utils::fs::get_404_output_file, MDBook};
use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    path::PathBuf,
};
use tower_http::services::{ServeDir, ServeFile};

/// The HTTP endpoint for the websocket used to trigger reloads when a file changes.
const LIVE_RELOAD_ENDPOINT: &str = "/__livereload";

/// Basic mdbook server. Given a path, hostname and port, serves the mdbook.
#[derive(Debug)]
pub struct Server {
    path: PathBuf,
    hostname: String,
    port: usize,
    open: bool,
}

impl Default for Server {
    fn default() -> Self {
        Self { path: PathBuf::default(), hostname: "localhost".to_owned(), port: 3000, open: false }
    }
}

impl Server {
    /// Create a new instance.
    pub fn new(path: PathBuf) -> Self {
        Self { path, ..Default::default() }
    }

    /// Set the host to serve on.
    pub fn with_hostname(mut self, hostname: String) -> Self {
        self.hostname = hostname;
        self
    }

    /// Set the port to serve on.
    pub fn with_port(mut self, port: usize) -> Self {
        self.port = port;
        self
    }

    /// Set whether to open the browser after serving.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Serve the mdbook.
    pub fn serve(self) -> eyre::Result<()> {
        let mut book =
            MDBook::load(&self.path).map_err(|err| eyre::eyre!("failed to load book: {err:?}"))?;

        let reload = LIVE_RELOAD_ENDPOINT.strip_prefix('/').unwrap();
        book.config.set("output.html.live-reload-endpoint", reload).unwrap();
        // Override site-url for local serving of the 404 file
        book.config.set("output.html.site-url", "/").unwrap();

        book.build().map_err(|err| eyre::eyre!("failed to build book: {err:?}"))?;

        let address = format!("{}:{}", self.hostname, self.port);
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

        let serving_url = format!("http://{address}");
        println!("Serving on: {serving_url}");

        let thread_handle = std::thread::spawn(move || serve(build_dir, sockaddr, &file_404));

        if self.open {
            open(serving_url);
        }

        match thread_handle.join() {
            Ok(r) => r.map_err(Into::into),
            Err(e) => std::panic::resume_unwind(e),
        }
    }
}

#[allow(clippy::needless_return)]
#[tokio::main]
async fn serve(build_dir: PathBuf, address: SocketAddr, file_404: &str) -> io::Result<()> {
    let file_404 = build_dir.join(file_404);
    let svc = ServeDir::new(build_dir).not_found_service(ServeFile::new(file_404));
    let app = Router::new().nest_service("/", get_service(svc));
    let tcp_listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(tcp_listener, app.into_make_service()).await
}

fn open<P: AsRef<std::ffi::OsStr>>(path: P) {
    info!("Opening web browser");
    if let Err(e) = opener::open(path) {
        error!("Error opening web browser: {}", e);
    }
}
