use std::path::PathBuf;

use clap::{Parser, ValueHint};

#[derive(Debug, Clone, Parser)]
pub struct InspectArgs {
    #[clap(help = "the path to the contract to flatten", value_hint = ValueHint::FilePath)]
    pub target_path: PathBuf,

    #[clap(long, short, help = "output path for the flattened contract", value_hint = ValueHint::FilePath)]
    pub output: Option<PathBuf>,

    #[clap(flatten)]
    core_flatten_args: CoreFlattenArgs,
}
