use eyre::Result;

pub use solar::config::LspArgs;

pub async fn run(args: LspArgs) -> Result<()> {
    solar_lsp::run_server_stdio(args).await?;
    Ok(())
}
