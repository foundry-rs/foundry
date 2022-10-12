use crate::{parser::ParsedSnippet, session::ChiselSession};
use solang_parser::diagnostics::Diagnostic;
use std::{error, error::Error, str::FromStr};
use strum::{EnumIter, IntoEnumIterator};
use yansi::Paint;

/// Prompt arrow slice
static PROMPT_ARROW: char = '➜';
/// Command leader character
static COMMAND_LEADER: char = '!';

/// The Chisel Dispatcher
#[derive(Debug, Default)]
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the prompt given the last input's error status
    pub fn get_prompt(&self) -> String {
        format!(
            "{} ",
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
                self.session.snippets.clear();
                println!("{}", Paint::green("Cleared current session!"))
            }
            ChiselCommand::Flush => {
                if let Err(e) = self.session.write() {
                    return DispatchResult::FileIoError(e.into())
                }
            }
            ChiselCommand::Load(_) => {
                // Use args as the name
                let name = args[0];
                if let Err(e) = self.session.write() {
                    return DispatchResult::FileIoError(e.into())
                }
                let new_session = match name {
                    "latest" => ChiselSession::latest(),
                    _ => ChiselSession::load(name),
                };

                // WARNING: Overwrites the current session
                if let Ok(new_session) = new_session {
                    self.session = new_session;
                } else {
                    return DispatchResult::CommandFailed(format!(
                        "{}: Failed to load session!",
                        Paint::red("⚒️ Chisel Error")
                    ))
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
                        Paint::red("⚒️ Chisel Error: No sessions found.")
                    ))
                }
            },
            ChiselCommand::Source => {
                return DispatchResult::CommandSuccess(Some(self.session.contract_source()))
            }
        }

        DispatchResult::CommandSuccess(None)
    }

    /// Dispatches an input to the appropriate chisel handlers
    ///
    /// ### Returns
    ///
    /// A DispatchResult
    pub fn dispatch(&mut self, line: &str) -> DispatchResult {
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

        // Parse the input with [solang-parser](https://docs.rs/solang-parser/latest/solang_parser)
        // Print dianostics and continue on error
        // If parsing successful, grab the (source unit, comment) tuple

        // TODO: This does check if the line is parsed successfully, but does
        // not check if the line conflicts with any previous declarations
        // (i.e. "uint a = 1;" could be declared twice). Should check against
        // the whole temp file so that previous inputs persist.
        let mut parsed_snippet = ParsedSnippet::new(line);
        if let Err(e) = parsed_snippet.parse() {
            self.errored = true;
            return DispatchResult::SolangParserFailed(e)
        }
        self.session.snippets.push(parsed_snippet);

        // Get a reference to the temp project
        let project = match self.session.project.as_ref().ok_or_else(|| {
            DispatchResult::Failure(Some(format!(
                "{}",
                Paint::red("⚒️ Chisel Error: Missing project configuration.")
            )))
        }) {
            Ok(project) => project,
            Err(e) => return e,
        };

        if project.add_source("REPL", self.session.contract_source()).is_ok() {
            match self.session.execute() {
                Ok(_) => DispatchResult::Success(Some(format!("{:?}", project.sources_path()))),
                Err(e) => {
                    let err = DispatchResult::Failure(Some(format!(
                        "{}",
                        Paint::red(format!("⚒️ Chisel Error: {}", e))
                    )));
                    // Remove the snippet that caused the compilation error
                    self.session.snippets.pop();
                    err
                }
            }
        } else {
            DispatchResult::Failure(Some(format!(
                "{}",
                Paint::red("⚒️ Chisel Error: Failed writing source file to temp project.")
            )))
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
    Load(String),
    /// List all cached sessions
    ListSessions,
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
            "load" => Ok(ChiselCommand::Load("latest".to_string())),
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
            ChiselCommand::Load(_) => ("load", "Load a previous session from cache"),
            ChiselCommand::ListSessions => ("list", "List all cached sessions"),
        }
    }
}
