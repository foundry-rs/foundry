//! Snapshot command

use crate::cmd::Cmd;
use std::{path::PathBuf, str::FromStr};
use structopt::StructOpt;

#[derive(Debug, Clone, StructOpt)]
pub struct Snapshot {
    #[structopt(flatten)]
    filter: SnapshotFilter,
    #[structopt(help = "Print to stdout.", short = "v")]
    verbose: bool,
    #[structopt(
        help = "Compare against a snapshot and display changes from the snapshot. Takes an optional snapshot file, [default: .gas_snapshot]",
        long
    )]
    diff: Option<Option<PathBuf>>,
    #[structopt(help = "How to format the output.", short, long)]
    format: Option<Format>,
    #[structopt(
        help = "Output file for the snapshot.",
        default_value = ".gas_snapshot",
        long,
        short
    )]
    output: PathBuf,
}

impl Cmd for Snapshot {
    fn run(self) -> eyre::Result<()> {
        dbg!(self);
        todo!()
    }
}

#[derive(Debug, Clone)]
pub enum Format {
    Table,
}

impl FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "t" | "table" => Ok(Format::Table),
            _ => Err(format!("Unrecognized format `{}`", s)),
        }
    }
}

#[derive(Debug, Clone, StructOpt)]
pub struct CheckSnapshot {
    #[structopt(
        help = " Input gas snapshot file to compare against.",
        default_value = ".gas_snapshot",
        short,
        long
    )]
    input: PathBuf,
    #[structopt(flatten)]
    filter: SnapshotFilter,
}

impl Cmd for CheckSnapshot {
    fn run(self) -> eyre::Result<()> {
        dbg!(self);
        todo!()
    }
}

#[derive(Debug, Clone, StructOpt, Default)]
pub struct SnapshotFilter {
    #[structopt(help = "A regular expression used for filtering tests.")]
    pattern: Option<String>,
    #[structopt(help = "Only include tests that used less gas that the given amount.", long)]
    min: Option<usize>,
    #[structopt(help = "Only include tests that used less gas that the given amount.", long)]
    max: Option<usize>,
}
