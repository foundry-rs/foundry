use crate::env::ChiselEnv;
use ansi_term::Color::{Cyan, Green, Red};
use std::{error, str::FromStr};
use strum::{EnumIter, IntoEnumIterator};

/// Custom Chisel commands
#[derive(Debug, EnumIter)]
pub enum ChiselCommand {
    Help,
    Source,
    Clear,
}

/// A command descriptor type
type CmdDescriptor = (&'static str, &'static str);

/// Custom Chisel command implementations
#[allow(unused)]
impl ChiselCommand {
    pub fn dispatch(&self, args: &[&str], env: &mut ChiselEnv) {
        match self {
            ChiselCommand::Help => {
                println!("{}", Cyan.paint("⚒️ Chisel help"));
                ChiselCommand::iter().for_each(|cmd| {
                    let descriptor = CmdDescriptor::from(cmd);
                    println!("!{} - {}", Green.paint(descriptor.0), descriptor.1);
                });
            }
            ChiselCommand::Source => println!("{}", env.contract_source()),
            ChiselCommand::Clear => {
                env.session.clear();
                println!("Cleared current session.");
            }
        }
    }
}

/// Attempt to convert a string slice to a `ChiselCommand`
impl FromStr for ChiselCommand {
    type Err = Box<dyn error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "help" => Ok(ChiselCommand::Help),
            "source" => Ok(ChiselCommand::Source),
            "clear" => Ok(ChiselCommand::Clear),
            _ => Err(Red
                .paint(format!("Unknown command \"{}\"! See available commands with `!help`.", s))
                .to_string()
                .into()),
        }
    }
}

/// Convert a `ChiselCommand` into a `CmdDescriptor` tuple
impl From<ChiselCommand> for CmdDescriptor {
    fn from(cmd: ChiselCommand) -> Self {
        match cmd {
            ChiselCommand::Help => ("help", "Display all commands"),
            ChiselCommand::Source => {
                ("source", "Display the source code of the current REPL session")
            }
            ChiselCommand::Clear => ("clear", "Clear the current session"),
        }
    }
}
