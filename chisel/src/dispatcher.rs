use crate::{prelude::SolidityHelper, session::ChiselSession, session_source::SessionSourceConfig};
use solang_parser::diagnostics::Diagnostic;
use std::{error, error::Error, str::FromStr};
use strum::{EnumIter, IntoEnumIterator};
use yansi::Paint;

/// Prompt arrow slice
static PROMPT_ARROW: char = '➜';
/// Command leader character
static COMMAND_LEADER: char = '!';

/// The Chisel Dispatcher
#[derive(Debug)]
pub struct ChiselDisptacher {
    /// The status of the previous dispatch
    pub errored: bool,
    /// A Chisel Session
    pub session: ChiselSession,
}

/// A Chisel Dispatch Result
#[derive(Debug)]
pub enum DispatchResult {
    /// A Generic Dispatch Success
    Success(Option<String>),
    /// A Generic Failure
    Failure(Option<String>),
    /// A successful ChiselCommand Execution
    CommandSuccess(Option<String>),
    /// A failure to parse a Chisel Command
    UnrecognizedCommand(Box<dyn Error>),
    /// The solang parser failed
    SolangParserFailed(Vec<Diagnostic>),
    /// A Command Failed with error message
    CommandFailed(String),
    /// File IO Error
    FileIoError(Box<dyn Error>),
}

impl ChiselDisptacher {
    /// Associated public function to create a new Dispatcher instance
    pub fn new(config: &SessionSourceConfig) -> Self {
        ChiselDisptacher { errored: false, session: ChiselSession::new(config) }
    }

    /// Returns the prompt given the last input's error status
    pub fn get_prompt(&self) -> String {
        format!(
            "{}{} ",
            if let Some(id) = self.session.id {
                format!("({}) ", format!("{}: {}", Paint::cyan("ID"), Paint::yellow(id)))
            } else {
                String::default()
            },
            if self.errored { Paint::red(PROMPT_ARROW) } else { Paint::green(PROMPT_ARROW) }
        )
    }

    /// Dispatches a ChiselCommand
    pub fn dispatch_command(&mut self, cmd: ChiselCommand, args: &[&str]) -> DispatchResult {
        match cmd {
            ChiselCommand::Help => {
                return DispatchResult::CommandSuccess(Some(format!(
                    "{}\n{}",
                    Paint::cyan("⚒️ Chisel help"),
                    ChiselCommand::iter()
                        .map(|cmd| {
                            let descriptor = CmdDescriptor::from(cmd);
                            format!("!{} - {}", Paint::green(descriptor.0), descriptor.1)
                        })
                        .collect::<Vec<String>>()
                        .join("\n")
                )))
            }
            ChiselCommand::Clear => {
                if let Some(session_source) = self.session.session_source.as_mut() {
                    // Drain all source sections
                    session_source.drain_run();
                    session_source.drain_global_code();
                    session_source.drain_top_level_code();

                    return DispatchResult::Success(Some(String::from("Cleared session!")));
                } else {
                    return DispatchResult::CommandFailed(
                        Paint::red("Session source not present!").to_string(),
                    );
                }
            }
            ChiselCommand::Flush => {
                if let Err(e) = self.session.write() {
                    return DispatchResult::FileIoError(e.into());
                }
                return DispatchResult::Success(Some(String::from(format!("Saved session to cache with ID = {}", self.session.id.unwrap()))))
            }
            ChiselCommand::Load => {
                if args.len() != 1 {
                    return DispatchResult::CommandFailed(format!("{}", Paint::red("⚒️ Chisel Error: Must supply a session ID as the argument.")))
                }

                // Use args as the name
                let name = args[0];
                // Try to save the current session before loading another
                if let Some(session_source) = &self.session.session_source {
                    // Don't save an empty session
                    if !session_source.run_code.is_empty() {
                        if let Err(e) = self.session.write() {
                            return DispatchResult::FileIoError(e.into());
                        }
                    }
                }
                // Parse the arguments
                let new_session = match name {
                    "latest" => ChiselSession::latest(),
                    _ => ChiselSession::load(name),
                };

                // WARNING: Overwrites the current session
                if let Ok(new_session) = new_session {
                    self.session = new_session;
                    return DispatchResult::CommandSuccess(Some(format!("Loaded Chisel session! (ID = {})", self.session.id.unwrap())))
                } else {
                    return DispatchResult::CommandFailed(format!(
                        "{}: Failed to load session!",
                        Paint::red("⚒️ Chisel Error")
                    ));
                }
            }
            ChiselCommand::ListSessions => match ChiselSession::list_sessions() {
                Ok(sessions) => {
                    return DispatchResult::CommandSuccess(Some(format!(
                        "{}\n{}",
                        Paint::cyan("⚒️ Chisel sessions"),
                        sessions
                            .iter()
                            .map(|(time, name)| {
                                format!("{} - {}", Paint::blue(format!("{:?}", time)), name)
                            })
                            .collect::<Vec<String>>()
                            .join("\n")
                    )))
                }
                Err(_) => {
                    return DispatchResult::CommandFailed(format!(
                        "{}",
                        Paint::red("⚒️ Chisel Error: No sessions found. Use the `!flush` command to save a session.")
                    ))
                }
            },
            ChiselCommand::Source => {
                return DispatchResult::CommandSuccess(Some(SolidityHelper::highlight(&self.session.contract_source())))
            }
            ChiselCommand::ClearCache => {
                match ChiselSession::clear_cache() {
                    Ok(_) => return DispatchResult::CommandSuccess(Some(String::from("Cleared chisel cache!"))),
                    Err(_) => {
                        return DispatchResult::CommandFailed(format!(
                            "{}",
                            Paint::red("⚒️ Chisel Error: Failed to clear cache!")
                        ));
                    }
                }
            }
            ChiselCommand::Fork => {
                if let Some(session_source) = self.session.session_source.as_mut() {
                    if args.len() == 0 {
                        session_source.config.evm_opts.fork_url = None;
                        return DispatchResult::Success(Some(String::from("Now using local environment.")))
                    } else if args.len() != 1 {
                        return DispatchResult::CommandFailed(format!("{}", Paint::red("⚒️ Chisel Error: Must supply a session ID as the argument.")))
                    }

                    // Set the fork URL in the current session source to the first argument
                    session_source.config.evm_opts.fork_url = Some(args[0].to_owned());
                    // Clear the backend so that it is re-instantiated with the new fork
                    // upon the next execution of the session source.
                    session_source.config.backend = None;

                    DispatchResult::Success(Some(format!("Successfully forked {}", args[0])))
                } else {
                    DispatchResult::CommandFailed(format!("{}", Paint::red("⚒️ Chisel Error: Must supply a session ID as the argument.")))
                }
            }
        }
    }

    /// Dispatches an input to the appropriate chisel handlers
    ///
    /// ### Returns
    ///
    /// A DispatchResult
    pub async fn dispatch(&mut self, line: &str) -> DispatchResult {
        // Check if the input is a builtin command.
        // Commands are denoted with a `!` leading character.
        if line.starts_with(COMMAND_LEADER) {
            let split: Vec<&str> = line.split(' ').collect();
            let raw_cmd = &split[0][1..];

            return match raw_cmd.parse::<ChiselCommand>() {
                Ok(cmd) => {
                    let command_dispatch = self.dispatch_command(cmd, &split[1..]);
                    self.errored = !matches!(command_dispatch, DispatchResult::CommandSuccess(_));
                    return command_dispatch
                }
                Err(e) => {
                    self.errored = true;
                    DispatchResult::UnrecognizedCommand(e)
                }
            }
        }

        // Get a reference to the session source
        let source = match self.session.session_source.as_mut().ok_or_else(|| {
            DispatchResult::Failure(Some(format!(
                "{}",
                Paint::red("⚒️ Chisel Error: Missing project configuration.")
            )))
        }) {
            Ok(project) => project,
            Err(e) => {
                self.errored = true;
                return e
            }
        };

        // TODO: Support function calls / expressions
        if let Some(generated_output) = &source.generated_output {
            if generated_output.intermediate.variable_definitions.get(line).is_some() {
                match source.inspect(line).await {
                    Ok(res) => {
                        self.errored = false;
                        return DispatchResult::Success(Some(res))
                    }
                    Err(e) => {
                        self.errored = true;
                        return DispatchResult::CommandFailed(e.to_string())
                    }
                }
            }
        }

        // Create new source and parse
        let mut new_source = match source.clone_with_new_line(line.to_string()) {
            Ok(new) => new,
            Err(e) => {
                self.errored = true;
                return DispatchResult::CommandFailed(e.to_string())
            }
        };

        match new_source.execute().await {
            Ok(res) => {
                let _res = res.1;
                self.session.session_source = Some(new_source);
                self.errored = false;
                DispatchResult::Success(None)
            }
            Err(e) => {
                self.errored = true;
                DispatchResult::CommandFailed(e.to_string())
            }
        }
    }
}

/// Custom Chisel commands
#[derive(Debug, EnumIter)]
pub enum ChiselCommand {
    /// Print helpful information about chisel
    Help,
    /// Clear the current session
    Clear,
    /// Print the generated source contract
    Source,
    /// Flush the current session to cache
    /// NOTE: This is not necessary as the session will be written to cache automatically
    Flush,
    /// Load a previous session from cache
    /// Requires a session name
    /// WARNING: This will overwrite the current session (though the current session will be
    /// optimistically cached)
    Load,
    /// List all cached sessions
    ListSessions,
    /// Clear the cache
    ClearCache,
    /// Fork an RPC
    Fork,
}

/// A command descriptor type
type CmdDescriptor = (&'static str, &'static str);

/// Attempt to convert a string slice to a `ChiselCommand`
impl FromStr for ChiselCommand {
    type Err = Box<dyn error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "help" => Ok(ChiselCommand::Help),
            "clear" => Ok(ChiselCommand::Clear),
            "source" => Ok(ChiselCommand::Source),
            "flush" => Ok(ChiselCommand::Flush),
            "list" => Ok(ChiselCommand::ListSessions),
            "load" => Ok(ChiselCommand::Load),
            "clearcache" => Ok(ChiselCommand::ClearCache),
            "fork" => Ok(ChiselCommand::Fork),
            _ => Err(Paint::red(format!(
                "Unknown command \"{}\"! See available commands with `!help`.",
                s
            ))
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
            ChiselCommand::Clear => ("clear", "Clear current session"),
            ChiselCommand::Source => {
                ("source", "Display the source code of the current REPL session")
            }
            ChiselCommand::Flush => ("flush", "Flush the current session to cache"),
            ChiselCommand::Load => ("load", "Load a previous session from cache"),
            ChiselCommand::ListSessions => ("list", "List all cached sessions"),
            ChiselCommand::ClearCache => ("clearcache", "Clear the chisel cache"),
            ChiselCommand::Fork => {
                ("fork", "Fork an RPC on-the-fly. Supply 0 arguments to return to a local network.")
            }
        }
    }
}
