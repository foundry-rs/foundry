//! Subcommands for forge

pub mod build;
pub mod snapshot;
pub mod test;
pub mod verify;

/// Common trait for all cli commands
pub trait Cmd: structopt::StructOpt + Sized {
    type Output;
    fn run(self) -> eyre::Result<Self::Output>;
}
