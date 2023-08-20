use clap::Parser;
use ethers::solc::{
    resolver::{Charset, TreeOptions},
    Graph,
};
use eyre::Result;
use foundry_cli::{opts::ProjectPathsArgs, utils::LoadConfig};

/// CLI arguments for `forge tree`.
#[derive(Debug, Clone, Parser)]
pub struct TreeArgs {
    /// Do not de-duplicate (repeats all shared dependencies)
    #[clap(long)]
    no_dedupe: bool,

    /// Character set to use in output.
    ///
    /// [possible values: utf8, ascii]
    #[clap(long, default_value = "utf8")]
    charset: Charset,

    #[clap(flatten)]
    opts: ProjectPathsArgs,
}

foundry_config::impl_figment_convert!(TreeArgs, opts);

impl TreeArgs {
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;
        let graph = Graph::resolve(&config.project_paths())?;
        let opts = TreeOptions { charset: self.charset, no_dedupe: self.no_dedupe };
        graph.print_with_options(opts);

        Ok(())
    }
}
