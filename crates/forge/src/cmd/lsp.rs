use clap::Parser;
use eyre::Result;

/// Start the Solar language server.
#[derive(Clone, Debug, Parser)]
pub struct LspArgs;

impl LspArgs {
    pub async fn run(self) -> Result<()> {
        // We ignore the error, see [`ErrorGuaranteed`] in `solar`.
        let _ = solar_lsp::run_server_stdio().await;

        Ok(())
    }
}
