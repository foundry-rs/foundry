use eyre::Result;
use foundry_config::Config;

pub use solar::config::LspArgs;

pub async fn run(args: LspArgs) -> Result<()> {
    let config = solar_lsp::LaunchConfig::from(args)
        .with_default_forge_path(std::env::current_exe()?)
        .with_selected_profile(Config::selected_profile().to_string());
    solar_lsp::launch(config).await?;
    Ok(())
}
