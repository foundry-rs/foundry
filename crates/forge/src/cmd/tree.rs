use clap::Parser;
use eyre::Result;
use foundry_cli::{install, opts::ProjectPathOpts, utils::LoadConfig};
use foundry_compilers::{
    Graph,
    resolver::{Charset, TreeOptions},
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
    project_paths: ProjectPathOpts,
}

foundry_config::impl_figment_convert!(TreeArgs, project_paths);

impl TreeArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;
        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            config = self.load_config()?;
        }
        let graph = <Graph>::resolve(&config.project_paths())?;
        let opts = TreeOptions { charset: self.charset, no_dedupe: self.no_dedupe };
        graph.print_with_options(opts);

        Ok(())
    }
}
