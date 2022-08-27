use crate::{
    cmd::{Cmd},
};
use clap::{Parser, ValueHint};
use foundry_config::{impl_figment_convert_basic};
use std::{
    path::{PathBuf},
};

#[derive(Debug, Clone, Parser)]
pub struct GeigerArgs {
    #[clap(
        help = "The project's root path.",
        long_help = "The project's root path. By default, this is the root directory of the current Git repository, or the current working directory.",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    root: Option<PathBuf>,
    #[clap(
        help = "Path to the config file.",
        long = "config-path",
        value_hint = ValueHint::FilePath,
        value_name = "FILE"
    )]
    config_path: Option<PathBuf>,
    #[clap(
        help = "path to a file or directory to detect",
        conflicts_with = "root",
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
        multiple = true
    )]
    paths: Vec<PathBuf>,
    #[clap(
        help = "name(s) of the dependencies to scan",
        conflicts_with = "paths",
        value_name = "DIR_NAME",
        multiple = true
    )]
    libs: Vec<String>,
    #[clap(
        help = "run in 'check' mode. Exits with 0 if no unsafe cheat codes were found. Exits with 1 if unsafe cheat codes are detected.",
        long
    )]
    check: bool,
}

impl_figment_convert_basic!(GeigerArgs);

// === impl GeigerArgs ===

impl GeigerArgs {
}

impl Cmd for GeigerArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        Ok(())
    }
}
