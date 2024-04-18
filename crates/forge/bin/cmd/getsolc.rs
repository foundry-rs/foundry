use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_compilers::Graph;
use foundry_config::Config;
use std::path::PathBuf;
use yansi::Paint;

/// CLI arguments for `forge get-solc`.
#[derive(Clone, Debug, Parser)]
pub struct GetSolcArgs {
    /// The path of the project to get solc for.
    #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
    root: PathBuf,
}

impl GetSolcArgs {
    pub fn run(self) -> Result<()> {
        let GetSolcArgs { root } = self;
        let config = Config::load_with_root(root);
        let project = config.project()?;
        let graph = Graph::resolve(&config.project_paths())?;
        let (versions, _) = graph.into_sources_by_version(project.offline)?;
        let sources_by_version = versions.get(&project)?;
        let versions: Vec<_> =
            sources_by_version.keys().map(|solc| solc.version_short().unwrap()).collect();
        if versions.len() == 1 {
            let version = &versions[0];
            println!(
                "{} {}.{}.{}",
                Paint::green("Solc"),
                version.major,
                version.minor,
                version.patch
            );
        } else {
            let formatted_versions: Vec<_> = versions
                .iter()
                .map(|version| format!("{}.{}.{}", version.major, version.minor, version.patch))
                .collect();
            let versions_str = formatted_versions.join(", ");
            println!("{} {}", Paint::green("Solcs"), versions_str);
        }
        Ok(())
    }
}
