use super::build::ProjectPathsArgs;
use clap::Parser;
use ethers::solc::{
    resolver::{Charset, TreeOptions},
    Graph,
};
use foundry_cli::utils::{Cmd, LoadConfig};

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

impl Cmd for TreeArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        let graph = Graph::resolve(&config.project_paths())?;
        let opts = TreeOptions { charset: self.charset, no_dedupe: self.no_dedupe };
        graph.print_with_options(opts);

        Ok(())
    }
}
