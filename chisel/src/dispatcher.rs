//! Dispatcher
//!
//! This module contains the `ChiselDispatcher` struct, which handles the dispatching
//! of both builtin commands and Solidity snippets.

use crate::{
    prelude::SolidityHelper, runner::ChiselResult, session::ChiselSession,
    session_source::SessionSourceConfig,
};
use ethers::utils::hex;
use forge::trace::{
    identifier::{EtherscanIdentifier, SignaturesIdentifier},
    CallTraceDecoder, CallTraceDecoderBuilder, TraceKind,
};
use foundry_config::Config;
use solang_parser::diagnostics::Diagnostic;
use std::{error::Error, path::PathBuf, str::FromStr};
use strum::{EnumIter, IntoEnumIterator};
use yansi::Paint;

/// Prompt arrow slice
static PROMPT_ARROW: char = '➜';
/// Command leader character
static COMMAND_LEADER: char = '!';
/// Chisel character
static CHISEL_CHAR: &str = "⚒️";

/// Chisel input dispatcher
#[derive(Debug)]
pub struct ChiselDisptacher {
    /// The status of the previous dispatch
    pub errored: bool,
    /// A Chisel Session
    pub session: ChiselSession,
}

/// Chisel dispatch result variants
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

    /// Dispatches a [ChiselCommand]
    pub async fn dispatch_command(&mut self, cmd: ChiselCommand, args: &[&str]) -> DispatchResult {
        match cmd {
            ChiselCommand::Help => {
                return DispatchResult::CommandSuccess(Some(format!(
                    "{}\n{}",
                    Paint::cyan(format!("{} Chisel help", CHISEL_CHAR)),
                    ChiselCommand::iter()
                        .map(|cmd| {
                            let (cmd, desc) = CmdDescriptor::from(cmd);
                            format!("!{} - {}", Paint::green(cmd), desc)
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

                    return DispatchResult::CommandSuccess(Some(String::from("Cleared session!")))
                } else {
                    return DispatchResult::CommandFailed(
                        Paint::red("Session source not present!").to_string(),
                    )
                }
            }
            ChiselCommand::Flush => {
                if let Err(e) = self.session.write() {
                    return DispatchResult::FileIoError(e.into())
                }
                return DispatchResult::CommandSuccess(Some(String::from(format!(
                    "Saved session to cache with ID = {}",
                    self.session.id.unwrap()
                ))))
            }
            ChiselCommand::Load => {
                if args.len() != 1 {
                    // Must supply a session ID as the argument.
                    return DispatchResult::CommandFailed(Self::make_error(
                        "Must supply a session ID as the argument.",
                    ))
                }

                // Use args as the name
                let name = args[0];
                // Try to save the current session before loading another
                if let Some(session_source) = &self.session.session_source {
                    // Don't save an empty session
                    if !session_source.run_code.is_empty() {
                        if let Err(e) = self.session.write() {
                            return DispatchResult::FileIoError(e.into())
                        }
                        println!("{}", Paint::green("Saved current session!"));
                    }
                }
                // Parse the arguments
                let new_session = match name {
                    "latest" => ChiselSession::latest(),
                    _ => ChiselSession::load(name),
                };

                // WARNING: Overwrites the current session
                if let Ok(mut new_session) = new_session {
                    // Regenerate [IntermediateOutput]; It cannot be serialized.
                    //
                    // SAFETY
                    // Should never panic due to the checks performed when the session was created
                    // in the first place.
                    new_session.session_source.as_mut().unwrap().build().unwrap();

                    self.session = new_session;
                    return DispatchResult::CommandSuccess(Some(format!(
                        "Loaded Chisel session! (ID = {})",
                        self.session.id.unwrap()
                    )))
                } else {
                    return DispatchResult::CommandFailed(Self::make_error(
                        "Failed to load session!",
                    ))
                }
            }
            ChiselCommand::ListSessions => match ChiselSession::list_sessions() {
                Ok(sessions) => {
                    return DispatchResult::CommandSuccess(Some(format!(
                        "{}\n{}",
                        Paint::cyan(format!("{} Chisel Sessions", CHISEL_CHAR)),
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
                    return DispatchResult::CommandFailed(Self::make_error(
                        "No sessions found. Use the `!flush` command to save a session.",
                    ))
                }
            },
            ChiselCommand::Source => {
                return DispatchResult::CommandSuccess(Some(SolidityHelper::highlight(
                    &self.session.contract_source(),
                )))
            }
            ChiselCommand::ClearCache => match ChiselSession::clear_cache() {
                Ok(_) => {
                    self.session.id = None;
                    return DispatchResult::CommandSuccess(Some(String::from(
                        "Cleared chisel cache!",
                    )))
                }
                Err(_) => {
                    return DispatchResult::CommandFailed(Self::make_error("Failed to clear cache!"))
                }
            },
            ChiselCommand::Fork => {
                if let Some(session_source) = self.session.session_source.as_mut() {
                    if args.len() == 0 {
                        session_source.config.evm_opts.fork_url = None;
                        return DispatchResult::CommandSuccess(Some(String::from(
                            "Now using local environment.",
                        )))
                    } else if args.len() != 1 {
                        return DispatchResult::CommandFailed(Self::make_error(
                            "Must supply a session ID as the argument.",
                        ))
                    }

                    // If the argument is an RPC alias designated in the
                    // `[rpc_endpoints]` section of the `foundry.toml` within
                    // the pwd, use the URL matched to the key.
                    let fork_url = if let Some(fork_url) =
                        session_source.config.config.rpc_endpoints.get(args[0])
                    {
                        fork_url.as_url().unwrap()
                    } else {
                        args[0]
                    };

                    // Update the fork_url inside of the [SessionSourceConfig]'s [EvmOpts]
                    // field
                    session_source.config.evm_opts.fork_url = Some(fork_url.to_owned());

                    // Clear the backend so that it is re-instantiated with the new fork
                    // upon the next execution of the session source.
                    session_source.config.backend = None;

                    DispatchResult::CommandSuccess(Some(format!(
                        "Set fork URL to {}",
                        Paint::yellow(fork_url)
                    )))
                } else {
                    DispatchResult::CommandFailed(Self::make_error("Session not present."))
                }
            }
            ChiselCommand::Traces => {
                if let Some(session_source) = self.session.session_source.as_mut() {
                    session_source.config.traces = !session_source.config.traces;
                    DispatchResult::CommandSuccess(Some(format!(
                        "Successfully {} traces!",
                        if session_source.config.traces { "enabled" } else { "disabled" }
                    )))
                } else {
                    DispatchResult::CommandFailed(Self::make_error("Session not present."))
                }
            }
            ChiselCommand::MemDump | ChiselCommand::StackDump => {
                if let Some(session_source) = self.session.session_source.as_mut() {
                    match session_source.execute().await {
                        Ok((_, res)) => {
                            if let Some((stack, mem, _)) = res.state.as_ref() {
                                if matches!(cmd, ChiselCommand::MemDump) {
                                    (0..mem.len()).step_by(32).for_each(|i| {
                                        println!(
                                            "{}: {}",
                                            Paint::yellow(format!(
                                                "[0x{:02x}:0x{:02x}]",
                                                i,
                                                i + 32
                                            )),
                                            Paint::cyan(format!(
                                                "0x{}",
                                                hex::encode(&mem.data()[i..i + 32])
                                            ))
                                        );
                                    });
                                } else {
                                    (0..stack.len()).rev().for_each(|i| {
                                        println!(
                                            "{}: {}",
                                            Paint::yellow(format!("[{}]", stack.len() - i - 1)),
                                            Paint::cyan(format!("0x{:02x}", stack.data()[i]))
                                        );
                                    });
                                }
                                DispatchResult::CommandSuccess(None)
                            } else {
                                DispatchResult::CommandFailed(Self::make_error(
                                    "State not present.",
                                ))
                            }
                        }
                        Err(e) => DispatchResult::CommandFailed(Self::make_error(e.to_string())),
                    }
                } else {
                    DispatchResult::CommandFailed(Self::make_error("Session not present."))
                }
            }
            ChiselCommand::Export => {
                // Check if the pwd is a foundry project
                if PathBuf::from("foundry.toml").exists() {
                    // Create "script" dir if it does not already exist.
                    if !PathBuf::from("script").exists() {
                        if let Err(e) = std::fs::create_dir_all("script") {
                            return DispatchResult::CommandFailed(Self::make_error(e.to_string()))
                        }
                    }
                    // Write session source to `script/REPL`
                    if let Err(e) = std::fs::write(
                        PathBuf::from("script/REPL.sol"),
                        self.session.session_source.as_ref().unwrap().to_string(),
                    ) {
                        return DispatchResult::CommandFailed(Self::make_error(e.to_string()))
                    }

                    DispatchResult::CommandSuccess(Some(String::from(
                        "Exported session source to script/REPL.sol!",
                    )))
                } else {
                    DispatchResult::CommandFailed(Self::make_error(
                        "Must be in a foundry project to export source to script.",
                    ))
                }
            }
        }
    }

    /// Dispatches an input as a command via [Self::dispatch_command] or as a Solidity snippet.
    pub async fn dispatch(&mut self, input: &str) -> DispatchResult {
        // Check if the input is a builtin command.
        // Commands are denoted with a `!` leading character.
        if input.starts_with(COMMAND_LEADER) {
            let split: Vec<&str> = input.split(' ').collect();
            let raw_cmd = &split[0][1..];

            return match raw_cmd.parse::<ChiselCommand>() {
                Ok(cmd) => {
                    let command_dispatch = self.dispatch_command(cmd, &split[1..]).await;
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
            DispatchResult::Failure(Some(Self::make_error("Session source missing!")))
        }) {
            Ok(project) => project,
            Err(e) => {
                self.errored = true;
                return e
            }
        };

        // TODO: Support expressions with ambiguous types / no variable declaration
        if let Some(generated_output) = &source.generated_output {
            if generated_output.intermediate.variable_definitions.get(input).is_some() {
                match source.inspect(input).await {
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
        let (mut new_source, do_execute) = match source.clone_with_new_line(input.to_string()) {
            Ok(new) => new,
            Err(e) => {
                self.errored = true;
                return DispatchResult::CommandFailed(Self::make_error(format!(
                    "Failed to parse input! {e}"
                )))
            }
        };

        if do_execute {
            match new_source.execute().await {
                Ok((_, mut res)) => {
                    let failed = !res.success;

                    // If traces are enabled or there was an error in execution, show the execution
                    // traces.
                    if new_source.config.traces || failed {
                        if let Ok(decoder) = self.decode_traces(&new_source.config, &mut res) {
                            if self.show_traces(&decoder, &mut res).await.is_err() {
                                self.errored = true;
                                return DispatchResult::CommandFailed(
                                    "Failed to display traces".to_owned(),
                                )
                            };

                            // If the contract execution failed, continue on without adding the new
                            // line to the source.
                            if failed {
                                self.errored = true;
                                return DispatchResult::Failure(Some(Self::make_error(
                                    "Failed to execute REPL contract!",
                                )))
                            }
                        }
                    }

                    // Replace the old session source with the new version
                    self.session.session_source = Some(new_source);
                    // Clear any outstanding errors
                    self.errored = false;

                    DispatchResult::Success(None)
                }
                Err(e) => {
                    self.errored = true;
                    DispatchResult::Failure(Some(e.to_string()))
                }
            }
        } else {
            match new_source.build() {
                Ok(_) => {
                    self.session.session_source = Some(new_source);
                    self.errored = false;
                    DispatchResult::Success(None)
                }
                Err(e) => DispatchResult::Failure(Some(e.to_string())),
            }
        }
    }

    /// Decodes traces in the [ChiselResult]
    /// TODO: Add `known_contracts` back in.
    pub fn decode_traces(
        &self,
        session_config: &SessionSourceConfig,
        result: &mut ChiselResult,
        // known_contracts: &ContractsByArtifact,
    ) -> eyre::Result<CallTraceDecoder> {
        let mut etherscan_identifier = EtherscanIdentifier::new(
            &session_config.config,
            session_config.evm_opts.get_remote_chain_id(),
        )?;

        let mut decoder =
            CallTraceDecoderBuilder::new().with_labels(result.labeled_addresses.clone()).build();

        decoder.add_signature_identifier(SignaturesIdentifier::new(
            Config::foundry_cache_dir(),
            session_config.config.offline,
        )?);

        for (_, trace) in &mut result.traces {
            // decoder.identify(trace, &mut local_identifier);
            decoder.identify(trace, &mut etherscan_identifier);
        }
        Ok(decoder)
    }

    /// Display the gathered traces of a REPL execution.
    pub async fn show_traces(
        &self,
        decoder: &CallTraceDecoder,
        result: &mut ChiselResult,
    ) -> eyre::Result<()> {
        if result.traces.is_empty() {
            eyre::bail!("Unexpected error: No traces gathered. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
        }

        println!("{}", Paint::green("Traces:"));
        for (kind, trace) in &mut result.traces {
            // Display all Setup + Execution traces.
            let should_include = match kind {
                TraceKind::Setup | TraceKind::Execution => true,
                _ => false,
            };

            if should_include {
                decoder.decode(trace).await;
                println!("{trace}");
            }
        }

        Ok(())
    }

    /// Format a type that implements [fmt::Display] as a chisel error string.
    fn make_error<T: std::fmt::Display>(msg: T) -> String {
        format!("{} {}", Paint::red(format!("{} Chisel Error:", CHISEL_CHAR)), Paint::red(msg))
    }
}

/// Builtin chisel command variants
#[derive(Debug, EnumIter)]
pub enum ChiselCommand {
    /// Print helpful information about chisel
    Help,
    /// Clear the current session source
    Clear,
    /// Print the generated source contract
    Source,
    /// Flush the current session to the cache
    Flush,
    /// Load a previous session from cache
    /// Requires a session id as the first argument
    /// WARNING: This will overwrite the current session (though the current session will be
    /// optimistically cached)
    Load,
    /// List all cached sessions
    ListSessions,
    /// Clear the cache of all stored sessions
    ClearCache,
    /// Fork an RPC in the current session
    Fork,
    /// Enable / disable traces for the current session
    Traces,
    /// Dump the raw memory
    MemDump,
    /// Dump the raw stack
    StackDump,
    /// Export the current REPL session source to a Script file
    Export,
}

/// A command descriptor type
type CmdDescriptor = (&'static str, &'static str);

/// Attempt to convert a string slice to a `ChiselCommand`
impl FromStr for ChiselCommand {
    type Err = Box<dyn Error>;

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
            "traces" => Ok(ChiselCommand::Traces),
            "memdump" => Ok(ChiselCommand::MemDump),
            "stackdump" => Ok(ChiselCommand::StackDump),
            "export" => Ok(ChiselCommand::Export),
            _ => Err(ChiselDisptacher::make_error(&format!(
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
                ("source", "Display the source code of the current session")
            }
            ChiselCommand::Flush => ("flush", "Flush the current session to cache"),
            ChiselCommand::Load => ("load", "Load a previous session ID from cache"),
            ChiselCommand::ListSessions => ("list", "List all cached sessions"),
            ChiselCommand::ClearCache => ("clearcache", "Clear the chisel cache of all stored sessions"),
            ChiselCommand::Fork => {
                ("fork", "Fork an RPC for the current session. Supply 0 arguments to return to a local network.")
            }
            ChiselCommand::Traces => ("traces", "Enable / disable traces for the current session"),
            ChiselCommand::MemDump => ("memdump", "Dump the raw memory of the current state"),
            ChiselCommand::StackDump => ("stackdump", "Dump the raw stack of the current state"),
            ChiselCommand::Export => ("export", "Export the current session source to a script file"),
        }
    }
}
