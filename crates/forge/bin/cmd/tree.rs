use clap::Parser;
use eyre::Result;
use foundry_cli::{opts::ProjectPathsArgs, utils::LoadConfig};
use foundry_compilers::{
    resolver::{parse::SolData, Charset, TreeOptions},
    Graph,
};

/// CLI arguments for `forge tree`.
#[derive(Clone, Debug, Parser)]
pub struct TreeArgs {
    /// Do not de-duplicate (repeats all shared dependencies)
    #[arg(long)]
    no_dedupe: bool,

    /// Character set to use in output.
    ///
    /// [possible values: utf8, ascii]
    #[arg(long, default_value = "utf8")]
    charset: Charset,

    #[command(flatten)]
    opts: ProjectPathsArgs,
}

foundry_config::impl_figment_convert!(TreeArgs, opts);

impl TreeArgs {
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;
        let graph = Graph::<SolData>::resolve(&config.project_paths())?;
        let opts = TreeOptions { charset: self.charset, no_dedupe: self.no_dedupe };
        graph.print_with_options(opts);

        Ok(())
    }
}
