use clap::Parser;
use eyre::Result;
use forge_lsp::ForgeLspServer;

/// Start the Foundry Language Server Protocol (LSP) server
#[derive(Clone, Debug, Parser)]
pub struct LspArgs {
    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,
}

impl LspArgs {
    pub async fn run(self) -> Result<()> {
        // Set up logging level based on debug flag
        if self.debug {
            unsafe {
                std::env::set_var("RUST_LOG", "debug");
            }
        }

        // Start the LSP server
        ForgeLspServer::run().await
    }
}
