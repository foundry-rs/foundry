use crate::env::ChiselEnv;
use ansi_term::Color::{Blue, Cyan, Red};
use std::{error, str::FromStr};
use strum::{EnumIter, IntoEnumIterator};
use yansi::Paint;

/// Custom Chisel commands
#[derive(Debug, EnumIter)]
pub enum ChiselCommand {
    /// Print helpful information about chisel
    Help,
    /// Print the generated source contract
    Source,
    /// Clears the current session
    Clear,
    /// Flush the current session to cache
    /// NOTE: This is not necessary as the session will be written to cache automatically
    Flush,
    /// Load a previous session from cache
    /// Requires a session name
    /// WARNING: This will overwrite the current session (though the current session will be
    /// optimistically cached)
    Load(String),
    /// List all cached sessions
    ListSessions,
}

/// A command descriptor type
type CmdDescriptor = (&'static str, &'static str);

/// Custom Chisel command implementations
#[allow(unused)]
impl ChiselCommand {
    /// Dispatches the chisel command to the appropriate handler/logic
    pub fn dispatch(&self, args: &[&str], env: &mut ChiselEnv) {
        match self {
            ChiselCommand::Help => {
                println!("{}", Paint::cyan("⚒️ Chisel help"));
                ChiselCommand::iter().for_each(|cmd| {
                    let descriptor = CmdDescriptor::from(cmd);
                    println!("!{} - {}", Paint::green(descriptor.0), descriptor.1);
                });
            }
            ChiselCommand::Flush => {
                env.write();
            }
            ChiselCommand::Load(name) => {
                env.write();
                let new_env = match name.as_str() {
                    "latest" => ChiselEnv::latest(),
                    _ => ChiselEnv::load(name),
                };

                // WARNING: Overwrites the current session
                if let Ok(new_env) = new_env {
                    *env = new_env;
                } else {
                    println!("{}: Failed to load session!", Red.paint("⚒️ Chisel Error"));
                }
            }
            ChiselCommand::ListSessions => match ChiselEnv::list_sessions() {
                Ok(sessions) => {
                    println!("{}", Cyan.paint("⚒️ Chisel sessions"));
                    sessions.iter().for_each(|(time, name)| {
                        println!("{} - {}", Blue.paint(format!("{:?}", time)), name);
                    });
                }
                Err(e) => {
                    println!("{}", Red.paint("⚒️ Chisel Error: No sessions found."));
                    println!("!{}", Red.paint(e.to_string()));
                }
            },
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
            "flush" => Ok(ChiselCommand::Flush),
            "list" => Ok(ChiselCommand::ListSessions),
            "load" => Ok(ChiselCommand::Load("latest".to_string())),
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
            ChiselCommand::Flush => ("flush", "Flush the current session to cache"),
            ChiselCommand::Load(_) => ("load", "Load a previous session from cache"),
            ChiselCommand::ListSessions => ("list", "List all cached sessions"),
        }
    }
}
