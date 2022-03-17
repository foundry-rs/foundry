//! tree command

use crate::cmd::{build::BuildArgs, Cmd};
use clap::Parser;
use ethers::solc::Graph;
use foundry_config::Config;

foundry_config::impl_figment_convert!(TreeArgs, opts);
use ethers::solc::resolver::{Charset, TreeOptions};

/// Command to display the project's dependency tree
#[derive(Debug, Clone, Parser)]
pub struct TreeArgs {
    // TODO extract path related args from BuildArgs
    #[clap(flatten)]
    opts: BuildArgs,
    #[clap(help = "Do not de-duplicate (repeats all shared dependencies)", long)]
    no_dedupe: bool,
    #[clap(help = "Character set to use in output: utf8, ascii", default_value = "utf8", long)]
    charset: Charset,
}

impl Cmd for TreeArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config: Config = From::from(&self);
        let graph = Graph::resolve(&config.project_paths())?;
        let opts = TreeOptions { charset: self.charset, no_dedupe: self.no_dedupe };
        graph.print_with_options(opts);

        Ok(())
    }
}
