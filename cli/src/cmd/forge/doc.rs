use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use forge_doc::{ContractInheritance, DocBuilder, Server};
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

    #[clap(
        help = "The doc's output path.",
        long_help = "The path where the docs are gonna get generated. By default, this is gonna be the docs directory at the root of the project.",
        long = "out",
        short,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    out: Option<PathBuf>,

    #[clap(help = "Serve the documentation.", long, short)]
    serve: bool,
}

impl Cmd for DocArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let root = self.root.clone().unwrap_or(find_project_root_path()?);
        let config = load_config_with_root(self.root.clone());
        let out = self.out.clone().unwrap_or(config.doc.out.clone());

        DocBuilder::new(root, config.project_paths().sources)
            .with_out(out.clone())
            .with_title(config.doc.title.clone())
            .with_preprocessor(ContractInheritance)
            .build()?;

        if self.serve {
            Server::new(out).serve()?;
        }

        Ok(())
    }
}
