use eyre::Result;
use foundry_compilers::compilers::multi::MultiCompilerLanguage;
use foundry_config::{Config, load_config_with_root};
use solar::config::ImportRemapping;
use solar_lsp::FoundryWorkspaceConfig;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

pub use solar::config::LspArgs;

pub async fn run(args: LspArgs) -> Result<()> {
    let config = solar_lsp::LaunchConfig::from(args)
        .with_default_forge_path(std::env::current_exe()?)
        .with_selected_profile(Config::selected_profile().to_string())
        .with_foundry_workspace_config_loader(|root| {
            foundry_workspace_config(root, load_config_with_root(Some(root))?)
        });
    solar_lsp::launch(config).await?;
    Ok(())
}

fn foundry_workspace_config(root: &Path, config: Config) -> Result<FoundryWorkspaceConfig> {
    let paths = config.project_paths::<MultiCompilerLanguage>();
    let resolved_root = paths.root;
    let sources = rebase_workspace_path(&resolved_root, root, paths.sources);
    let tests = rebase_workspace_path(&resolved_root, root, paths.tests);
    let scripts = rebase_workspace_path(&resolved_root, root, paths.scripts);

    Ok(FoundryWorkspaceConfig::new(root)
        .with_source_roots([sources.clone()])
        .with_flycheck_source_roots([sources, tests, scripts])
        .with_include_paths(
            paths
                .libraries
                .into_iter()
                .chain(paths.include_paths)
                .map(|path| rebase_workspace_path(&resolved_root, root, path)),
        )
        .with_import_remappings(paths.remappings.into_iter().map(|remapping| ImportRemapping {
            context: rebase_remapping_path(
                &resolved_root,
                root,
                remapping.context.unwrap_or_default(),
            ),
            prefix: remapping.name,
            path: rebase_remapping_path(&resolved_root, root, remapping.path),
        }))
        .with_evm_version(config.evm_version.to_string().parse()?))
}

fn rebase_workspace_path(resolved_root: &Path, root: &Path, path: PathBuf) -> PathBuf {
    let Ok(relative) = path.strip_prefix(resolved_root) else { return path };
    root.join(relative)
}

fn rebase_remapping_path(resolved_root: &Path, root: &Path, path: impl Into<String>) -> String {
    let path = path.into();
    let has_directory_boundary = path.ends_with(['/', '\\']);
    let mut rebased =
        rebase_workspace_path(resolved_root, root, PathBuf::from(path)).display().to_string();
    if has_directory_boundary && !rebased.ends_with(['/', '\\']) {
        rebased.push(MAIN_SEPARATOR);
    }
    rebased
}
