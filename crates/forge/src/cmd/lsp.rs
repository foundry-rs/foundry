use clap::Parser;
use eyre::Result;
use foundry_compilers::compilers::multi::MultiCompilerLanguage;
use foundry_config::{Config, load_config_with_root};
use solar::config::ImportRemapping;
use solar_lsp::FoundryWorkspaceConfig;
use std::{
    io::IsTerminal,
    path::{MAIN_SEPARATOR, Path, PathBuf},
};

mod editor;

/// Open a Solidity project in VS Code or run its language server.
#[derive(Debug, Default, Parser)]
pub struct LspArgs {
    /// Run the language server over standard input/output.
    #[arg(long, conflicts_with_all = ["vscode", "path", "code_path"])]
    pub stdio: bool,

    /// Open a VS Code Extension Development Host even when input is redirected.
    #[arg(long)]
    pub vscode: bool,

    /// Project directory to open in VS Code. Defaults to the current directory.
    #[arg(value_hint = clap::ValueHint::DirPath)]
    pub path: Option<PathBuf>,

    /// VS Code executable or command. Defaults to `code` on PATH.
    #[arg(long, value_hint = clap::ValueHint::ExecutablePath)]
    pub code_path: Option<PathBuf>,
}

pub async fn run(args: LspArgs) -> Result<()> {
    if !args.stdio
        && (args.vscode
            || args.path.is_some()
            || args.code_path.is_some()
            || std::io::stdin().is_terminal())
    {
        return editor::launch(args.path.as_deref(), args.code_path.as_deref());
    }

    let config = solar_lsp::LaunchConfig::from(solar::config::LspArgs { stdio: args.stdio })
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
