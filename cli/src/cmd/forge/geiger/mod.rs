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
    /// Paths to files or directories to detect.
    #[clap(
        conflicts_with = "root",
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(1..),
    )]
    paths: Vec<PathBuf>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Run in "check" mode.
    ///
    /// The exit code of the program will be the number of unsafe cheatcodes found.
    #[clap(long)]
    pub check: bool,

    /// Globs to ignore.
    #[clap(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        num_args(1..),
    )]
    ignore: Vec<PathBuf>,

    /// Print a report of all files, even if no unsafe functions are found.
    #[clap(long)]
    full: bool,
}

impl_figment_convert_basic!(GeigerArgs);

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
    type Output = usize;

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        let sources = self.sources(&config).wrap_err("Failed to resolve files")?;

        if config.ffi {
            eprintln!("{}\n", Paint::red("ffi enabled"));
        }

        let root = config.__root.0;

        let sum = sources
            .par_iter()
            .map(|file| match find_cheatcodes_in_file(file) {
                Ok(metrics) => {
                    let len = metrics.cheatcodes.len();
                    let printer = SolFileMetricsPrinter { metrics: &metrics, root: &root };
                    if self.full || len == 0 {
                        eprint!("{printer}");
                    }
                    len
                }
                Err(err) => {
                    eprintln!("{err}");
                    0
                }
            })
            .sum();

        Ok(sum)
    }
}
