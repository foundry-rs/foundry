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
    let solcs: Vec<_> =
        sources_by_version.keys().map(|solc| solc.version_short().unwrap()).collect();
    println!("found solc {}\n", Paint::green(format!("{:?}", solcs)));
    Ok(())
}
