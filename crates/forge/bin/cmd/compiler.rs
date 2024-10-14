use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_compilers::Graph;
use foundry_config::Config;
use std::path::PathBuf;

/// CLI arguments for `forge compiler`.
#[derive(Clone, Debug, Parser)]
pub enum CompilerSubcommands {
    /// Retrieves the resolved version(s) of the compiler within the project.
    #[command(visible_alias = "r")]
    Resolve {
        /// The root directory
        #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
        root: PathBuf,
    },
}

impl CompilerSubcommands {
    pub async fn run(self) -> Result<()> {
        match &self {
            Self::Resolve { root } => self.handle_version_resolving(root).await,
        }
    }

    /// Retrieves the resolved version(s) of the compiler within the project.
    async fn handle_version_resolving(&self, root: &PathBuf) -> Result<()> {
        let config = Config::load_with_root(root);
        let project = config.project()?;

        let graph = Graph::resolve(&project.paths)?;
        let (sources, _) = graph.into_sources_by_version(
            project.offline,
            &project.locked_versions,
            &project.compiler,
        )?;

        for (language, sources) in sources {
            for (version, sources) in sources {
                println!("{language} {version}:");
                for (idx, (path_file, _)) in sources.iter().enumerate() {
                    let path = path_file
                        .strip_prefix(&project.paths.root)
                        .unwrap_or(path_file)
                        .to_path_buf();
                    let prefix = if idx == sources.len() - 1 { "└──" } else { "├──" };
                    println!("{prefix} {}", path.display());
                }
            }
        }

        Ok(())
    }
}
