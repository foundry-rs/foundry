use eyre::Result;
use foundry_compilers::compilers::multi::MultiCompilerLanguage;
use foundry_config::{Config, load_config_with_root};
use solar::config::ImportRemapping;
use solar_lsp::FoundryWorkspaceConfig;

pub use solar::config::LspArgs;

pub async fn run(args: LspArgs) -> Result<()> {
    let config = solar_lsp::LaunchConfig::from(args)
        .with_default_forge_path(std::env::current_exe()?)
        .with_selected_profile(Config::selected_profile().to_string())
        .with_foundry_workspace_config_loader(|root| {
            foundry_workspace_config(load_config_with_root(Some(root))?)
        });
    solar_lsp::launch(config).await?;
    Ok(())
}

fn foundry_workspace_config(config: Config) -> Result<FoundryWorkspaceConfig> {
    let paths = config.project_paths::<MultiCompilerLanguage>();
    Ok(FoundryWorkspaceConfig::new(paths.root)
        .with_source_roots([paths.sources.clone()])
        .with_flycheck_source_roots([paths.sources, paths.tests, paths.scripts])
        .with_include_paths(paths.libraries.into_iter().chain(paths.include_paths))
        .with_import_remappings(paths.remappings.into_iter().map(|remapping| ImportRemapping {
            context: remapping.context.unwrap_or_default(),
            prefix: remapping.name,
            path: remapping.path,
        }))
        .with_evm_version(config.evm_version.to_string().parse()?))
}
