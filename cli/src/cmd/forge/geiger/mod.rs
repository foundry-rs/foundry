use crate::cmd::{
    forge::geiger::find::{find_cheatcodes_in_file, SolFileMetricsPrinter},
    Cmd, LoadConfig,
};
use clap::{Parser, ValueHint};
use ethers::solc::Graph;
use eyre::WrapErr;
use foundry_config::{impl_figment_convert_basic, Config};
use itertools::Itertools;
use rayon::prelude::*;
use std::path::PathBuf;
use yansi::Paint;

mod error;
mod find;
mod visitor;

/// CLI arguments for `forge geiger`.
#[derive(Debug, Clone, Parser)]
pub struct GeigerArgs {
    #[clap(
        help = "path to a file or directory to detect",
        conflicts_with = "root",
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(1..)
    )]
    paths: Vec<PathBuf>,
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    root: Option<PathBuf>,
    #[clap(
        help = "run in 'check' mode. Exits with 0 if no unsafe cheat codes were found. Exits with 1 if unsafe cheat codes are detected.",
        long
    )]
    check: bool,
    #[clap(
        help = "Globs to ignore.",
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(1..),
        long
    )]
    ignore: Vec<PathBuf>,
    #[clap(help = "print a full report of all files even if no unsafe functions are found.", long)]
    full: bool,
}

impl_figment_convert_basic!(GeigerArgs);

// === impl GeigerArgs ===

impl GeigerArgs {
    pub fn sources(&self, config: &Config) -> eyre::Result<Vec<PathBuf>> {
        let cwd = std::env::current_dir()?;

        let mut sources: Vec<PathBuf> = {
            if self.paths.is_empty() {
                Graph::resolve(&config.project_paths())?.files().keys().cloned().collect()
            } else {
                self.paths
                    .iter()
                    .flat_map(|path| foundry_common::fs::files_with_ext(path, "sol"))
                    .unique()
                    .collect()
            }
        };

        sources.retain(|path| {
            let abs_path = if path.is_absolute() { path.clone() } else { cwd.join(path) };
            !self.ignore.iter().any(|ignore| {
                if ignore.is_absolute() {
                    abs_path.starts_with(ignore)
                } else {
                    abs_path.starts_with(cwd.join(ignore))
                }
            })
        });

        Ok(sources)
    }
}

impl Cmd for GeigerArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        let sources = self.sources(&config).wrap_err("Failed to resolve files")?;

        if config.ffi {
            eprintln!("{}\n", Paint::red("ffi enabled"));
        }

        let root = config.__root.0;

        sources.par_iter().map(|file| find_cheatcodes_in_file(file)).for_each(|res| {
            match res {
                Ok(metrics) => {
                    let printer = SolFileMetricsPrinter { metrics: &metrics, root: &root };
                    if self.full || printer.metrics.cheatcodes.has_unsafe() {
                        eprint!("{printer}");
                    }
                }
                Err(err) => {
                    eprintln!("{err}");
                }
            };
        });

        Ok(())
    }
}
