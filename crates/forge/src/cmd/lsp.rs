use clap::Parser;
use eyre::Result;

use forge_lsp::{analyzer::Analyzer, lsp::ForgeLsp};
use foundry_cli::{opts::BuildOpts, utils::LoadConfig};
use tower_lsp::{LspService, Server};
use tracing::info;

/// Start the Foundry Language Server Protocol (LSP) server
#[derive(Clone, Debug, Parser)]
pub struct LspArgs {
    /// See: <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#implementationConsiderations>
    #[arg(long)]
    pub stdio: bool,

    #[command(flatten)]
    pub build: BuildOpts,
}

foundry_config::impl_figment_convert!(LspArgs, build);

impl LspArgs {
    pub async fn run(self) -> Result<()> {
        let config = self.load_config()?;
        let build_opts = self.build;

        // Start stdio LSP server
        info!("Starting Foundry LSP server...");

        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) =
            LspService::new(move |client| ForgeLsp::new(client, Analyzer::new(config, build_opts)));

        // Run server
        Server::new(stdin, stdout, socket).serve(service).await;

        info!("Foundry LSP server stopped");

        Ok(())
    }
}
