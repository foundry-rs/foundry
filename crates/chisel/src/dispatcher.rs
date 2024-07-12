//! Dispatcher
//!
//! This module contains the `ChiselDispatcher` struct, which handles the dispatching
//! of both builtin commands and Solidity snippets.

use crate::{
    prelude::{
        ChiselCommand, ChiselResult, ChiselSession, CmdCategory, CmdDescriptor,
        SessionSourceConfig, SolidityHelper,
    },
    session_source::SessionSource,
};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{hex, Address};
use forge_fmt::FormatterConfig;
use foundry_config::{Config, RpcEndpoint};
use foundry_evm::{
    decode::decode_console_logs,
    traces::{
        decode_trace_arena,
        identifier::{SignaturesIdentifier, TraceIdentifiers},
        render_trace_arena, CallTraceDecoder, CallTraceDecoderBuilder, TraceKind,
    },
};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use solang_parser::diagnostics::Diagnostic;
use std::{
    borrow::Cow,
    error::Error,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use strum::IntoEnumIterator;
use tracing::debug;
use yansi::Paint;

/// Prompt arrow character
pub static PROMPT_ARROW: char = '➜';
static DEFAULT_PROMPT: &str = "➜ ";

/// Command leader character
pub static COMMAND_LEADER: char = '!';
/// Chisel character
pub static CHISEL_CHAR: &str = "⚒️";

/// Matches Solidity comments
static COMMENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(?://.*\s*$)|(/*[\s\S]*?\*/\s*$)").unwrap());

/// Matches Ethereum addresses that are not strings
static ADDRESS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)(([^"']\s*)|^)(?P<address>0x[a-fA-F0-9]{40})((\s*[^"'\w])|$)"#).unwrap()
});

/// Chisel input dispatcher
#[derive(Debug)]
pub struct ChiselDispatcher {
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

impl DispatchResult {
    /// Returns `true` if the result is an error.
    pub fn is_error(&self) -> bool {
        matches!(
            self,
            Self::Failure(_) |
                Self::CommandFailed(_) |
                Self::UnrecognizedCommand(_) |
                Self::SolangParserFailed(_) |
                Self::FileIoError(_)
        )
    }
}

/// A response from the Etherscan API's `getabi` action
#[derive(Debug, Serialize, Deserialize)]
pub struct EtherscanABIResponse {
    /// The status of the response
    /// "1" = success | "0" = failure
    pub status: String,
    /// The message supplied by the API
    pub message: String,
    /// The result returned by the API. Will be `None` if the request failed.
    pub result: Option<String>,
}

/// Used to format ABI parameters into valid solidity function / error / event param syntax
/// TODO: Smarter resolution of storage location, defaults to "memory" for all types
/// that cannot be stored on the stack.
macro_rules! format_param {
    ($param:expr) => {{
        let param = $param;
        format!("{}{}", param.ty, if param.is_complex_type() { " memory" } else { "" })
    }};
}

/// Helper function that formats solidity source with the given [FormatterConfig]
pub fn format_source(source: &str, config: FormatterConfig) -> eyre::Result<String> {
    match forge_fmt::parse(source) {
        Ok(parsed) => {
            let mut formatted_source = String::default();

            if forge_fmt::format_to(&mut formatted_source, parsed, config).is_err() {
                eyre::bail!("Could not format source!");
            }

            Ok(formatted_source)
        }
        Err(_) => eyre::bail!("Formatter could not parse source!"),
    }
}

impl ChiselDispatcher {
    /// Associated public function to create a new Dispatcher instance
    pub fn new(config: SessionSourceConfig) -> eyre::Result<Self> {
        ChiselSession::new(config).map(|session| Self { session })
    }

    /// Returns the optional ID of the current session.
    pub fn id(&self) -> Option<&str> {
        self.session.id.as_deref()
    }

    /// Returns the [`SessionSource`].
    pub fn source(&self) -> &SessionSource {
        &self.session.session_source
    }

    /// Returns the [`SessionSource`].
    pub fn source_mut(&mut self) -> &mut SessionSource {
        &mut self.session.session_source
    }

    fn format_source(&self) -> eyre::Result<String> {
        format_source(
            &self.source().to_repl_source(),
            self.source().config.foundry_config.fmt.clone(),
        )
    }

    /// Returns the prompt based on the current status of the Dispatcher
    pub fn get_prompt(&self) -> Cow<'static, str> {
        match self.session.id.as_deref() {
            // `(ID: {id}) ➜ `
            Some(id) => {
                let mut prompt = String::with_capacity(DEFAULT_PROMPT.len() + id.len() + 7);
                prompt.push_str("(ID: ");
                prompt.push_str(id);
                prompt.push_str(") ");
                prompt.push_str(DEFAULT_PROMPT);
                Cow::Owned(prompt)
            }
            // `➜ `
            None => Cow::Borrowed(DEFAULT_PROMPT),
        }
    }

    /// Dispatches a [ChiselCommand]
    ///
    /// ### Takes
    ///
    /// - A [ChiselCommand]
    /// - An array of arguments
    ///
    /// ### Returns
    ///
    /// A [DispatchResult] containing feedback on the dispatch's execution.
    pub async fn dispatch_command(&mut self, cmd: ChiselCommand, args: &[&str]) -> DispatchResult {
        match cmd {
            ChiselCommand::Help => {
                let all_descriptors =
                    ChiselCommand::iter().map(CmdDescriptor::from).collect::<Vec<CmdDescriptor>>();
                DispatchResult::CommandSuccess(Some(format!(
                    "{}\n{}",
                    format!("{CHISEL_CHAR} Chisel help\n=============").cyan(),
                    CmdCategory::iter()
                        .map(|cat| {
                            // Get commands in the current category
                            let cat_cmds = &all_descriptors
                                .iter()
                                .filter(|(_, _, c)| {
                                    std::mem::discriminant(c) == std::mem::discriminant(&cat)
                                })
                                .collect::<Vec<&CmdDescriptor>>();

                            // Format the help menu for the current category
                            format!(
                                "{}\n{}\n",
                                cat.magenta(),
                                cat_cmds
                                    .iter()
                                    .map(|(cmds, desc, _)| format!(
                                        "\t{} - {}",
                                        cmds.iter()
                                            .map(|cmd| format!("!{}", cmd.green()))
                                            .collect::<Vec<_>>()
                                            .join(" | "),
                                        desc
                                    ))
                                    .collect::<Vec<String>>()
                                    .join("\n")
                            )
                        })
                        .collect::<Vec<String>>()
                        .join("\n")
                )))
            }
            ChiselCommand::Quit => {
                // Exit the process with status code `0` for success.
                std::process::exit(0);
            }
            ChiselCommand::Clear => {
                self.source_mut().drain_run();
                self.source_mut().drain_global_code();
                self.source_mut().drain_top_level_code();
                DispatchResult::CommandSuccess(Some(String::from("Cleared session!")))
            }
            ChiselCommand::Save => {
                if args.len() <= 1 {
                    // If a new name was supplied, overwrite the ID of the current session.
                    if args.len() == 1 {
                        // TODO: Should we delete the old cache file if the id of the session
                        // changes?
                        self.session.id = Some(args[0].to_owned());
                    }

                    if let Err(e) = self.session.write() {
                        return DispatchResult::FileIoError(e.into())
                    }
                    DispatchResult::CommandSuccess(Some(format!(
                        "Saved session to cache with ID = {}",
                        self.session.id.as_ref().unwrap()
                    )))
                } else {
                    DispatchResult::CommandFailed(Self::make_error("Too many arguments supplied!"))
                }
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
                // Don't save an empty session
                if !self.source().run_code.is_empty() {
                    if let Err(e) = self.session.write() {
                        return DispatchResult::FileIoError(e.into())
                    }
                    println!("{}", "Saved current session!".green());
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
                    new_session.session_source.build().unwrap();

                    self.session = new_session;
                    DispatchResult::CommandSuccess(Some(format!(
                        "Loaded Chisel session! (ID = {})",
                        self.session.id.as_ref().unwrap()
                    )))
                } else {
                    DispatchResult::CommandFailed(Self::make_error("Failed to load session!"))
                }
            }
            ChiselCommand::ListSessions => match ChiselSession::list_sessions() {
                Ok(sessions) => DispatchResult::CommandSuccess(Some(format!(
                    "{}\n{}",
                    format!("{CHISEL_CHAR} Chisel Sessions").cyan(),
                    sessions
                        .iter()
                        .map(|(time, name)| {
                            format!("{} - {}", format!("{time:?}").blue(), name)
                        })
                        .collect::<Vec<String>>()
                        .join("\n")
                ))),
                Err(_) => DispatchResult::CommandFailed(Self::make_error(
                    "No sessions found. Use the `!save` command to save a session.",
                )),
            },
            ChiselCommand::Source => match self.format_source() {
                Ok(formatted_source) => DispatchResult::CommandSuccess(Some(
                    SolidityHelper::highlight(&formatted_source).into_owned(),
                )),
                Err(_) => {
                    DispatchResult::CommandFailed(String::from("Failed to format session source"))
                }
            },
            ChiselCommand::ClearCache => match ChiselSession::clear_cache() {
                Ok(_) => {
                    self.session.id = None;
                    DispatchResult::CommandSuccess(Some(String::from("Cleared chisel cache!")))
                }
                Err(_) => DispatchResult::CommandFailed(Self::make_error("Failed to clear cache!")),
            },
            ChiselCommand::Fork => {
                if args.is_empty() || args[0].trim().is_empty() {
                    self.source_mut().config.evm_opts.fork_url = None;
                    return DispatchResult::CommandSuccess(Some(
                        "Now using local environment.".to_string(),
                    ))
                }
                if args.len() != 1 {
                    return DispatchResult::CommandFailed(Self::make_error(
                        "Must supply a session ID as the argument.",
                    ))
                }
                let arg = *args.first().unwrap();

                // If the argument is an RPC alias designated in the
                // `[rpc_endpoints]` section of the `foundry.toml` within
                // the pwd, use the URL matched to the key.
                let endpoint = if let Some(endpoint) =
                    self.source_mut().config.foundry_config.rpc_endpoints.get(arg)
                {
                    endpoint.clone()
                } else {
                    RpcEndpoint::Env(arg.to_string()).into()
                };
                let fork_url = match endpoint.resolve() {
                    Ok(fork_url) => fork_url,
                    Err(e) => {
                        return DispatchResult::CommandFailed(Self::make_error(format!(
                            "\"{}\" ENV Variable not set!",
                            e.var
                        )))
                    }
                };

                // Check validity of URL
                if Url::parse(&fork_url).is_err() {
                    return DispatchResult::CommandFailed(Self::make_error("Invalid fork URL!"))
                }

                // Create success message before moving the fork_url
                let success_msg = format!("Set fork URL to {}", &fork_url.yellow());

                // Update the fork_url inside of the [SessionSourceConfig]'s [EvmOpts]
                // field
                self.source_mut().config.evm_opts.fork_url = Some(fork_url);

                // Clear the backend so that it is re-instantiated with the new fork
                // upon the next execution of the session source.
                self.source_mut().config.backend = None;

                DispatchResult::CommandSuccess(Some(success_msg))
            }
            ChiselCommand::Traces => {
                self.source_mut().config.traces = !self.source_mut().config.traces;
                DispatchResult::CommandSuccess(Some(format!(
                    "{} traces!",
                    if self.source_mut().config.traces { "Enabled" } else { "Disabled" }
                )))
            }
            ChiselCommand::Calldata => {
                // remove empty space, double quotes, and 0x prefix
                let arg = args
                    .first()
                    .map(|s| s.trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\''))
                    .map(|s| s.strip_prefix("0x").unwrap_or(s))
                    .unwrap_or("");

                if arg.is_empty() {
                    self.source_mut().config.calldata = None;
                    return DispatchResult::CommandSuccess(Some("Calldata cleared.".to_string()))
                }

                let calldata = hex::decode(arg);
                match calldata {
                    Ok(calldata) => {
                        self.source_mut().config.calldata = Some(calldata);
                        DispatchResult::CommandSuccess(Some(format!(
                            "Set calldata to '{}'",
                            arg.yellow()
                        )))
                    }
                    Err(e) => DispatchResult::CommandFailed(Self::make_error(format!(
                        "Invalid calldata: {e}"
                    ))),
                }
            }
            ChiselCommand::MemDump | ChiselCommand::StackDump => {
                match self.source_mut().execute().await {
                    Ok((_, res)) => {
                        if let Some((stack, mem, _)) = res.state.as_ref() {
                            if matches!(cmd, ChiselCommand::MemDump) {
                                // Print memory by word
                                (0..mem.len()).step_by(32).for_each(|i| {
                                    println!(
                                        "{}: {}",
                                        format!("[0x{:02x}:0x{:02x}]", i, i + 32).yellow(),
                                        hex::encode_prefixed(&mem[i..i + 32]).cyan()
                                    );
                                });
                            } else {
                                // Print all stack items
                                (0..stack.len()).rev().for_each(|i| {
                                    println!(
                                        "{}: {}",
                                        format!("[{}]", stack.len() - i - 1).yellow(),
                                        format!("0x{:02x}", stack[i]).cyan()
                                    );
                                });
                            }
                            DispatchResult::CommandSuccess(None)
                        } else {
                            DispatchResult::CommandFailed(Self::make_error(
                                "Run function is empty.",
                            ))
                        }
                    }
                    Err(e) => DispatchResult::CommandFailed(Self::make_error(e.to_string())),
                }
            }
            ChiselCommand::Export => {
                // Check if the current session inherits `Script.sol` before exporting

                // Check if the pwd is a foundry project
                if !Path::new("foundry.toml").exists() {
                    return DispatchResult::CommandFailed(Self::make_error(
                        "Must be in a foundry project to export source to script.",
                    ));
                }

                // Create "script" dir if it does not already exist.
                if !Path::new("script").exists() {
                    if let Err(e) = std::fs::create_dir_all("script") {
                        return DispatchResult::CommandFailed(Self::make_error(e.to_string()))
                    }
                }

                match self.format_source() {
                    Ok(formatted_source) => {
                        // Write session source to `script/REPL.s.sol`
                        if let Err(e) =
                            std::fs::write(PathBuf::from("script/REPL.s.sol"), formatted_source)
                        {
                            return DispatchResult::CommandFailed(Self::make_error(e.to_string()))
                        }

                        DispatchResult::CommandSuccess(Some(String::from(
                            "Exported session source to script/REPL.s.sol!",
                        )))
                    }
                    Err(_) => DispatchResult::CommandFailed(String::from(
                        "Failed to format session source",
                    )),
                }
            }
            ChiselCommand::Fetch => {
                if args.len() != 2 {
                    return DispatchResult::CommandFailed(Self::make_error(
                        "Incorrect number of arguments supplied. Expected: <address> <name>",
                    ))
                }

                let request_url = format!(
                    "https://api.etherscan.io/api?module=contract&action=getabi&address={}{}",
                    args[0],
                    if let Some(api_key) =
                        self.source().config.foundry_config.etherscan_api_key.as_ref()
                    {
                        format!("&apikey={api_key}")
                    } else {
                        String::default()
                    }
                );

                // TODO: Not the cleanest method of building a solidity interface from
                // the ABI, but does the trick. Might want to pull this logic elsewhere
                // and/or refactor at some point.
                match reqwest::get(&request_url).await {
                    Ok(response) => {
                        let json = response.json::<EtherscanABIResponse>().await.unwrap();
                        if json.status == "1" && json.result.is_some() {
                            let abi = json.result.unwrap();
                            let abi: serde_json::Result<JsonAbi> =
                                serde_json::from_slice(abi.as_bytes());
                            if let Ok(abi) = abi {
                                let mut interface = format!(
                                    "// Interface of {}\ninterface {} {{\n",
                                    args[0], args[1]
                                );

                                // Add error definitions
                                abi.errors().for_each(|err| {
                                    interface.push_str(&format!(
                                        "\terror {}({});\n",
                                        err.name,
                                        err.inputs
                                            .iter()
                                            .map(|input| format_param!(input))
                                            .collect::<Vec<_>>()
                                            .join(",")
                                    ));
                                });
                                // Add event definitions
                                abi.events().for_each(|event| {
                                    interface.push_str(&format!(
                                        "\tevent {}({});\n",
                                        event.name,
                                        event
                                            .inputs
                                            .iter()
                                            .map(|input| {
                                                let mut formatted = input.ty.to_string();
                                                if input.indexed {
                                                    formatted.push_str(" indexed");
                                                }
                                                formatted
                                            })
                                            .collect::<Vec<_>>()
                                            .join(",")
                                    ));
                                });
                                // Add function definitions
                                abi.functions().for_each(|func| {
                                    interface.push_str(&format!(
                                        "\tfunction {}({}) external{}{};\n",
                                        func.name,
                                        func.inputs
                                            .iter()
                                            .map(|input| format_param!(input))
                                            .collect::<Vec<_>>()
                                            .join(","),
                                        match func.state_mutability {
                                            alloy_json_abi::StateMutability::Pure => " pure",
                                            alloy_json_abi::StateMutability::View => " view",
                                            alloy_json_abi::StateMutability::Payable => " payable",
                                            _ => "",
                                        },
                                        if func.outputs.is_empty() {
                                            String::default()
                                        } else {
                                            format!(
                                                " returns ({})",
                                                func.outputs
                                                    .iter()
                                                    .map(|output| format_param!(output))
                                                    .collect::<Vec<_>>()
                                                    .join(",")
                                            )
                                        }
                                    ));
                                });
                                // Close interface definition
                                interface.push('}');

                                // Add the interface to the source outright - no need to verify
                                // syntax via compilation and/or
                                // parsing.
                                self.source_mut().with_global_code(&interface);

                                DispatchResult::CommandSuccess(Some(format!(
                                    "Added {}'s interface to source as `{}`",
                                    args[0], args[1]
                                )))
                            } else {
                                DispatchResult::CommandFailed(Self::make_error(
                                    "Contract is not verified!",
                                ))
                            }
                        } else if let Some(error_msg) = json.result {
                            DispatchResult::CommandFailed(Self::make_error(format!(
                                "Could not fetch interface - \"{error_msg}\""
                            )))
                        } else {
                            DispatchResult::CommandFailed(Self::make_error(format!(
                                "Could not fetch interface - \"{}\"",
                                json.message
                            )))
                        }
                    }
                    Err(e) => DispatchResult::CommandFailed(Self::make_error(format!(
                        "Failed to communicate with Etherscan API: {e}"
                    ))),
                }
            }
            ChiselCommand::Exec => {
                if args.is_empty() {
                    return DispatchResult::CommandFailed(Self::make_error("No command supplied!"))
                }

                let mut cmd = Command::new(args[0]);
                if args.len() > 1 {
                    cmd.args(args[1..].iter().copied());
                }

                match cmd.output() {
                    Ok(output) => {
                        std::io::stdout().write_all(&output.stdout).unwrap();
                        std::io::stdout().write_all(&output.stderr).unwrap();
                        DispatchResult::CommandSuccess(None)
                    }
                    Err(e) => DispatchResult::CommandFailed(e.to_string()),
                }
            }
            ChiselCommand::Edit => {
                // create a temp file with the content of the run code
                let mut temp_file_path = std::env::temp_dir();
                temp_file_path.push("chisel-tmp.sol");
                let result = std::fs::File::create(&temp_file_path)
                    .map(|mut file| file.write_all(self.source().run_code.as_bytes()));
                if let Err(e) = result {
                    return DispatchResult::CommandFailed(format!(
                        "Could not write to a temporary file: {e}"
                    ))
                }

                // open the temp file with the editor
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
                let mut cmd = Command::new(editor);
                cmd.arg(&temp_file_path);

                match cmd.status() {
                    Ok(status) => {
                        if !status.success() {
                            if let Some(status_code) = status.code() {
                                return DispatchResult::CommandFailed(format!(
                                    "Editor exited with status {status_code}"
                                ))
                            } else {
                                return DispatchResult::CommandFailed(
                                    "Editor exited without a status code".to_string(),
                                )
                            }
                        }
                    }
                    Err(_) => {
                        return DispatchResult::CommandFailed(
                            "Editor exited without a status code".to_string(),
                        )
                    }
                }

                let mut new_session_source = self.source().clone();
                if let Ok(edited_code) = std::fs::read_to_string(temp_file_path) {
                    new_session_source.drain_run();
                    new_session_source.with_run_code(&edited_code);
                } else {
                    return DispatchResult::CommandFailed(
                        "Could not read the edited file".to_string(),
                    )
                }

                // if the editor exited successfully, try to compile the new code
                match new_session_source.execute().await {
                    Ok((_, mut res)) => {
                        let failed = !res.success;
                        if new_session_source.config.traces || failed {
                            if let Ok(decoder) =
                                Self::decode_traces(&new_session_source.config, &mut res).await
                            {
                                if let Err(e) = Self::show_traces(&decoder, &mut res).await {
                                    return DispatchResult::CommandFailed(e.to_string())
                                };

                                // Show console logs, if there are any
                                let decoded_logs = decode_console_logs(&res.logs);
                                if !decoded_logs.is_empty() {
                                    println!("{}", "Logs:".green());
                                    for log in decoded_logs {
                                        println!("  {log}");
                                    }
                                }
                            }

                            // If the contract execution failed, continue on without
                            // updating the source.
                            DispatchResult::CommandFailed(Self::make_error(
                                "Failed to execute edited contract!",
                            ))
                        } else {
                            // the code could be compiled, save it
                            *self.source_mut() = new_session_source;
                            DispatchResult::CommandSuccess(Some(String::from(
                                "Successfully edited `run()` function's body!",
                            )))
                        }
                    }
                    Err(_) => {
                        DispatchResult::CommandFailed("The code could not be compiled".to_string())
                    }
                }
            }
            ChiselCommand::RawStack => {
                let len = args.len();
                if len != 1 {
                    let msg = match len {
                        0 => "No variable supplied!",
                        _ => "!rawstack only takes one argument.",
                    };
                    return DispatchResult::CommandFailed(Self::make_error(msg))
                }

                // Store the variable that we want to inspect
                let to_inspect = args.first().unwrap();

                // Get a mutable reference to the session source
                let source = self.source_mut();

                // Copy the variable's stack contents into a bytes32 variable without updating
                // the current session source.
                let line = format!("bytes32 __raw__; assembly {{ __raw__ := {to_inspect} }}");
                if let Ok((new_source, _)) = source.clone_with_new_line(line) {
                    match new_source.inspect("__raw__").await {
                        Ok((_, Some(res))) => return DispatchResult::CommandSuccess(Some(res)),
                        Ok((_, None)) => {}
                        Err(e) => return DispatchResult::CommandFailed(Self::make_error(e)),
                    }
                }

                DispatchResult::CommandFailed(
                    "Variable must exist within `run()` function.".to_string(),
                )
            }
        }
    }

    /// Dispatches an input as a command via [Self::dispatch_command] or as a Solidity snippet.
    pub async fn dispatch(&mut self, mut input: &str) -> DispatchResult {
        // Check if the input is a builtin command.
        // Commands are denoted with a `!` leading character.
        if input.starts_with(COMMAND_LEADER) {
            let split: Vec<&str> = input.split_whitespace().collect();
            let raw_cmd = &split[0][1..];

            return match raw_cmd.parse::<ChiselCommand>() {
                Ok(cmd) => self.dispatch_command(cmd, &split[1..]).await,
                Err(e) => DispatchResult::UnrecognizedCommand(e),
            }
        }
        if input.trim().is_empty() {
            debug!("empty dispatch input");
            return DispatchResult::Success(None)
        }

        // Get a mutable reference to the session source
        let source = self.source_mut();

        // If the input is a comment, add it to the run code so we avoid running with empty input
        if COMMENT_RE.is_match(input) {
            debug!(%input, "matched comment");
            source.with_run_code(input);
            return DispatchResult::Success(None)
        }

        // If there is an address (or multiple addresses) in the input, ensure that they are
        // encoded with a valid checksum per EIP-55.
        let mut heap_input = input.to_string();
        ADDRESS_RE.captures_iter(input).for_each(|m| {
            // Convert the match to a string slice
            let match_str = m.name("address").expect("exists").as_str();

            // We can always safely unwrap here due to the regex matching.
            let addr: Address = match_str.parse().expect("Valid address regex");
            // Replace all occurrences of the address with a checksummed version
            heap_input = heap_input.replace(match_str, &addr.to_string());
        });
        // Replace the old input with the formatted input.
        input = &heap_input;

        // Create new source with exact input appended and parse
        let (mut new_source, do_execute) = match source.clone_with_new_line(input.to_string()) {
            Ok(new) => new,
            Err(e) => {
                return DispatchResult::CommandFailed(Self::make_error(format!(
                    "Failed to parse input! {e}"
                )))
            }
        };

        // TODO: Cloning / parsing the session source twice on non-inspected inputs kinda sucks.
        // Should change up how this works.
        match source.inspect(input).await {
            // Continue and print
            Ok((true, Some(res))) => println!("{res}"),
            Ok((true, None)) => {}
            // Return successfully
            Ok((false, res)) => {
                debug!(%input, ?res, "inspect success");
                return DispatchResult::Success(res)
            }

            // Return with the error
            Err(e) => return DispatchResult::CommandFailed(Self::make_error(e)),
        }

        if do_execute {
            match new_source.execute().await {
                Ok((_, mut res)) => {
                    let failed = !res.success;

                    // If traces are enabled or there was an error in execution, show the execution
                    // traces.
                    if new_source.config.traces || failed {
                        if let Ok(decoder) = Self::decode_traces(&new_source.config, &mut res).await
                        {
                            if let Err(e) = Self::show_traces(&decoder, &mut res).await {
                                return DispatchResult::CommandFailed(e.to_string())
                            };

                            // Show console logs, if there are any
                            let decoded_logs = decode_console_logs(&res.logs);
                            if !decoded_logs.is_empty() {
                                println!("{}", "Logs:".green());
                                for log in decoded_logs {
                                    println!("  {log}");
                                }
                            }

                            // If the contract execution failed, continue on without adding the new
                            // line to the source.
                            if failed {
                                return DispatchResult::Failure(Some(Self::make_error(
                                    "Failed to execute REPL contract!",
                                )))
                            }
                        }
                    }

                    // Replace the old session source with the new version
                    *self.source_mut() = new_source;

                    DispatchResult::Success(None)
                }
                Err(e) => DispatchResult::Failure(Some(e.to_string())),
            }
        } else {
            match new_source.build() {
                Ok(out) => {
                    debug!(%input, ?out, "skipped execute and rebuild source");
                    *self.source_mut() = new_source;
                    DispatchResult::Success(None)
                }
                Err(e) => DispatchResult::Failure(Some(e.to_string())),
            }
        }
    }

    /// Decodes traces in the [ChiselResult]
    /// TODO: Add `known_contracts` back in.
    ///
    /// ### Takes
    ///
    /// - A reference to a [SessionSourceConfig]
    /// - A mutable reference to a [ChiselResult]
    ///
    /// ### Returns
    ///
    /// Optionally, a [CallTraceDecoder]
    pub async fn decode_traces(
        session_config: &SessionSourceConfig,
        result: &mut ChiselResult,
        // known_contracts: &ContractsByArtifact,
    ) -> eyre::Result<CallTraceDecoder> {
        let mut decoder = CallTraceDecoderBuilder::new()
            .with_labels(result.labeled_addresses.clone())
            .with_signature_identifier(SignaturesIdentifier::new(
                Config::foundry_cache_dir(),
                session_config.foundry_config.offline,
            )?)
            .build();

        let mut identifier = TraceIdentifiers::new().with_etherscan(
            &session_config.foundry_config,
            session_config.evm_opts.get_remote_chain_id().await,
        )?;
        if !identifier.is_empty() {
            for (_, trace) in &mut result.traces {
                decoder.identify(trace, &mut identifier);
            }
        }
        Ok(decoder)
    }

    /// Display the gathered traces of a REPL execution.
    ///
    /// ### Takes
    ///
    /// - A reference to a [CallTraceDecoder]
    /// - A mutable reference to a [ChiselResult]
    ///
    /// ### Returns
    ///
    /// Optionally, a unit type signifying a successful result.
    pub async fn show_traces(
        decoder: &CallTraceDecoder,
        result: &mut ChiselResult,
    ) -> eyre::Result<()> {
        if result.traces.is_empty() {
            eyre::bail!("Unexpected error: No traces gathered. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
        }

        println!("{}", "Traces:".green());
        for (kind, trace) in &mut result.traces {
            // Display all Setup + Execution traces.
            if matches!(kind, TraceKind::Setup | TraceKind::Execution) {
                decode_trace_arena(trace, decoder).await?;
                println!("{}", render_trace_arena(trace));
            }
        }

        Ok(())
    }

    /// Format a type that implements [std::fmt::Display] as a chisel error string.
    ///
    /// ### Takes
    ///
    /// A generic type implementing the [std::fmt::Display] trait.
    ///
    /// ### Returns
    ///
    /// A formatted error [String].
    pub fn make_error<T: std::fmt::Display>(msg: T) -> String {
        format!("{} {}", format!("{CHISEL_CHAR} Chisel Error:").red(), msg.red())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comment_regex() {
        assert!(COMMENT_RE.is_match("// line comment"));
        assert!(COMMENT_RE.is_match("  \n// line \tcomment\n"));
        assert!(!COMMENT_RE.is_match("// line \ncomment"));

        assert!(COMMENT_RE.is_match("/* block comment */"));
        assert!(COMMENT_RE.is_match(" \t\n  /* block \n \t comment */\n"));
        assert!(!COMMENT_RE.is_match("/* block \n \t comment */\nwith \tother"));
    }

    #[test]
    fn test_address_regex() {
        assert!(ADDRESS_RE.is_match("0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4"));
        assert!(ADDRESS_RE.is_match(" 0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4 "));
        assert!(ADDRESS_RE.is_match("0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4,"));
        assert!(ADDRESS_RE.is_match("(0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4)"));
        assert!(!ADDRESS_RE.is_match("0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4aaa"));
        assert!(!ADDRESS_RE.is_match("'0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4'"));
        assert!(!ADDRESS_RE.is_match("'    0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4'"));
        assert!(!ADDRESS_RE.is_match("'0xe5f3aF50FE5d0bF402a3C6F55ccC47d4307922d4'"));
    }
}
