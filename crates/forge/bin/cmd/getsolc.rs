use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_compilers::{artifacts::Source, utils, Graph, Project};
use std::path::PathBuf;
use yansi::Paint;

/// CLI arguments for `forge get-solc`.
#[derive(Clone, Debug, Parser)]
pub struct GetSolcArgs {
    /// The path of the project to get solc for.
    #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
    path: PathBuf,
}

impl GetSolcArgs {
    pub fn run(self) -> Result<()> {
        let GetSolcArgs { path } = self;
        get_solc_from_path(path)?;
        Ok(())
    }
}

pub fn get_solc_from_path(path: PathBuf) -> Result<()> {
    let files = utils::source_files(path);
    let sources = Source::read_all(files)?;
    let project = Project::builder().build()?;
    let graph = Graph::resolve_sources(&project.paths, sources)?;
    let (versions, _) = graph.into_sources_by_version(project.offline)?;
    let sources_by_version = versions.get(&project)?;
    let versions: Vec<_> =
        sources_by_version.keys().map(|solc| solc.version_short().unwrap()).collect();
    if versions.len() == 1 {
        let version = &versions[0];
        println!(
            "Solc {}",
            Paint::green(format!("{}.{}.{}", version.major, version.minor, version.patch))
        );
    } else {
        let formatted_versions: Vec<_> = versions
            .iter()
            .map(|version| {
                Paint::green(format!("{}.{}.{}", version.major, version.minor, version.patch))
                    .to_string()
            })
            .collect();
        let versions_str = formatted_versions.join(", ");
        println!("Solcs {}", versions_str);
    }
    Ok(())
}
