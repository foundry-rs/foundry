use eyre::Result;

pub use solar::config::LspArgs;

pub async fn run(args: LspArgs) -> Result<()> {
    let config =
        solar_lsp::LaunchConfig::from(args).with_default_forge_path(std::env::current_exe()?);
    solar_lsp::launch(config).await?;
    Ok(())
}
