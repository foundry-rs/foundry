use crate::lsp::ForgeLsp;
use eyre::Result;
use tower_lsp::{LspService, Server};
use tracing::info;

pub struct ForgeLspServer;

impl ForgeLspServer {
    pub async fn run() -> Result<()> {
        info!("Starting Foundry LSP server...");

        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(|client| ForgeLsp { client });

        Server::new(stdin, stdout, socket).serve(service).await;

        info!("Foundry LSP server stopped");

        Ok(())
    }
}
