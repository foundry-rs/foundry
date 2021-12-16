use std::path::PathBuf;
use structopt::{clap::AppSettings, StructOpt};

/// Main forge-shell args
#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[AppSettings::ColoredHelp])]
pub struct Args {
    #[structopt(
        help = "Select a series of libraries to pre load into session. These can be directories or single files."
    )]
    pub libs: Vec<PathBuf>,
    #[structopt(
        help = "Select a whole project to load into session during start up.",
        short = "w",
        long = "workspace"
    )]
    pub workspace: Option<PathBuf>,

    #[structopt(subcommand)]
    pub subcommand: Option<SubCommand>,
}

#[derive(Debug, StructOpt)]
pub enum SubCommand {
    // TODO want subcommands do we need?
}
