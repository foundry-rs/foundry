use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_compilers::{compilers::solc::SolcVersionManager, Graph};
use foundry_config::Config;
use std::path::PathBuf;

/// CLI arguments for `forge solc`.
#[derive(Clone, Debug, Parser)]
pub enum SolcSubcommands {
    /// Retrieves the resolved version(s) of the solidity compiler (solc) within the project.
    #[command(visible_alias = "vr")]
    VersionResolving {
        /// The root directory
        #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
        root: PathBuf,
    },
}

impl SolcSubcommands {
    pub async fn run(self) -> Result<()> {
        match &self {
            SolcSubcommands::VersionResolving { root } => {
                self.handle_version_resolving(&root).await
            }
        }
    }

    async fn handle_version_resolving(&self, root: &PathBuf) -> Result<()> {
        let config = Config::load_with_root(root);
        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;

        let version_manager = SolcVersionManager::default();
        let (versions, _) = graph.into_sources_by_version(project.offline, &version_manager)?;

        let sources_by_version = versions.get(&version_manager)?;

        for (_, version, sources) in sources_by_version {
            println!("{}", version);
            for (path_file, _) in sources.iter() {
                println!(
                    "├── {}",
                    path_file
                        .strip_prefix(&project.paths.root)
                        .unwrap_or(path_file)
                        .to_path_buf()
                        .display()
                );
            }
            println!("");
        }
        Ok(())
    }
}
