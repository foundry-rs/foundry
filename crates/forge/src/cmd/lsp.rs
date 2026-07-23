use clap::Parser;
use eyre::Result;

/// Start the Solar language server over standard input and output.
#[derive(Clone, Debug, Parser)]
pub struct LspArgs;

impl LspArgs {
    pub async fn run(self) -> Result<()> {
        solar_lsp::run_server_stdio(Default::default()).await?;
        Ok(())
    }
}
