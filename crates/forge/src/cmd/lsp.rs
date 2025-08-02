use clap::Parser;
use eyre::Result;

use forge_lsp::lsp::ForgeLsp;
use tower_lsp::{LspService, Server};
use tracing::info;

/// Start the Foundry Language Server Protocol (LSP) server
#[derive(Clone, Debug, Parser)]
pub struct LspArgs {
    /// See: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#implementationConsiderations
    #[arg(long)]
    pub stdio: bool,
}

impl LspArgs {
    pub async fn run(self) -> Result<()> {
        // Start stdio LSP server
        info!("Starting Foundry LSP server...");

        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(|client| ForgeLsp { client });

        Server::new(stdin, stdout, socket).serve(service).await;

        info!("Foundry LSP server stopped");

        Ok(())
    }
}
