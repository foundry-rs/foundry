use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use forge_doc::builder::DocBuilder;
use foundry_config::{find_project_root_path, load_config_with_root};
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
pub struct DocArgs {
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    root: Option<PathBuf>,
}

impl Cmd for DocArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = load_config_with_root(self.root.clone());
        DocBuilder::new(self.root.as_ref().unwrap_or(&find_project_root_path()?))
            .with_paths(config.project_paths().input_files())
            .build()
    }
}
