//! Dispatcher
//!
//! This module contains the `ChiselDispatcher` struct, which handles the dispatching
//! of both builtin commands and Solidity snippets.

use crate::{
    prelude::{ChiselCommand, ChiselResult, ChiselSession, SessionSourceConfig, SolidityHelper},
    source::SessionSource,
};
use alloy_primitives::{Address, hex};
use eyre::{Context, Result};
use forge_fmt::FormatterConfig;
use foundry_cli::utils::fetch_abi_from_etherscan;
use foundry_config::RpcEndpointUrl;
use foundry_evm::{
    decode::decode_console_logs,
    traces::{
        CallTraceDecoder, CallTraceDecoderBuilder, TraceKind, decode_trace_arena,
        identifier::{SignaturesIdentifier, TraceIdentifiers},
        render_trace_arena,
    },
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use solar::{
    parse::lexer::token::{RawLiteralKind, RawTokenKind},
    sema::ast::Base,
};
use std::{
    borrow::Cow,
    io::Write,
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::Builder;
use tracing::debug;
use yansi::Paint;

/// Prompt arrow character.
pub const PROMPT_ARROW: char = '➜';
/// Prompt arrow string.
pub const PROMPT_ARROW_STR: &str = "➜";
const DEFAULT_PROMPT: &str = "➜ ";

/// Command leader character
pub const COMMAND_LEADER: char = '!';
/// Chisel character
pub const CHISEL_CHAR: &str = "⚒️";

/// Chisel input dispatcher
#[derive(Debug)]
pub struct ChiselDispatcher {
    pub session: ChiselSession,
    pub helper: SolidityHelper,
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

/// Helper function that formats solidity source with the given [FormatterConfig]
pub fn format_source(source: &str, config: FormatterConfig) -> eyre::Result<String> {
    let formatted = forge_fmt::format(source, config).into_result()?;
    Ok(formatted)
}

impl ChiselDispatcher {
    /// Associated public function to create a new Dispatcher instance
    pub fn new(config: SessionSourceConfig) -> eyre::Result<Self> {
        let session = ChiselSession::new(config)?;
        Ok(Self { session, helper: Default::default() })
    }

    /// Returns the optional ID of the current session.
    pub fn id(&self) -> Option<&str> {
        self.session.id.as_deref()
    }

    /// Returns the [`SessionSource`].
    pub fn source(&self) -> &SessionSource {
        &self.session.source
    }

    /// Returns the [`SessionSource`].
    pub fn source_mut(&mut self) -> &mut SessionSource {
        &mut self.session.source
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

    /// Dispatches an input as a command via [Self::dispatch_command] or as a Solidity snippet.
    pub async fn dispatch(&mut self, mut input: &str) -> Result<ControlFlow<()>> {
        if let Some(command) = input.strip_prefix(COMMAND_LEADER) {
            return match ChiselCommand::parse(command) {
                Ok(cmd) => self.dispatch_command(cmd).await,
                Err(e) => eyre::bail!("unrecognized command: {e}"),
            };
        }

        let source = self.source_mut();

        input = input.trim();
        let (only_trivia, new_input) = preprocess(input);
        input = &*new_input;

        // If the input is a comment, add it to the run code so we avoid running with empty input
        if only_trivia {
            debug!(?input, "matched trivia");
            if !input.is_empty() {
                source.add_run_code(input);
            }
            return Ok(ControlFlow::Continue(()));
        }

        // Create new source with exact input appended and parse
        let (new_source, do_execute) = source.clone_with_new_line(input.to_string())?;

        // TODO: Cloning / parsing the session source twice on non-inspected inputs kinda sucks.
        // Should change up how this works.
        let (cf, res) = source.inspect(input).await?;
        if let Some(res) = &res {
            let _ = sh_println!("{res}");
        }
        if cf.is_break() {
            debug!(%input, ?res, "inspect success");
            return Ok(ControlFlow::Continue(()));
        }

        if do_execute {
            self.execute_and_replace(new_source).await.map(ControlFlow::Continue)
        } else {
            let out = new_source.build()?;
            debug!(%input, ?out, "skipped execute and rebuild source");
            *self.source_mut() = new_source;
            Ok(ControlFlow::Continue(()))
        }
    }

    /// Decodes traces in the given [`ChiselResult`].
    // TODO: Add `known_contracts` back in.
    pub async fn decode_traces(
        session_config: &SessionSourceConfig,
        result: &mut ChiselResult,
        // known_contracts: &ContractsByArtifact,
    ) -> eyre::Result<CallTraceDecoder> {
        let mut decoder = CallTraceDecoderBuilder::new()
            .with_labels(result.labeled_addresses.clone())
            .with_signature_identifier(SignaturesIdentifier::from_config(
                &session_config.foundry_config,
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
    pub async fn show_traces(
        decoder: &CallTraceDecoder,
        result: &mut ChiselResult,
    ) -> eyre::Result<()> {
        if result.traces.is_empty() {
            return Ok(());
        }

        sh_println!("{}", "Traces:".green())?;
        for (kind, trace) in &mut result.traces {
            // Display all Setup + Execution traces.
            if matches!(kind, TraceKind::Setup | TraceKind::Execution) {
                decode_trace_arena(trace, decoder).await;
                sh_println!("{}", render_trace_arena(trace))?;
            }
        }

        Ok(())
    }

    async fn execute_and_replace(&mut self, mut new_source: SessionSource) -> Result<()> {
        let (_, mut res) = new_source.execute().await?;
        let failed = !res.success;
        if new_source.config.traces || failed {
            if let Ok(decoder) = Self::decode_traces(&new_source.config, &mut res).await {
                Self::show_traces(&decoder, &mut res).await?;

                // Show console logs, if there are any
                let decoded_logs = decode_console_logs(&res.logs);
                if !decoded_logs.is_empty() {
                    let _ = sh_println!("{}", "Logs:".green());
                    for log in decoded_logs {
                        let _ = sh_println!("  {log}");
                    }
                }
            }

            if failed {
                // If the contract execution failed, continue on without
                // updating the source.
                eyre::bail!("Failed to execute edited contract!");
            }
        }

        // the code could be compiled, save it
        *self.source_mut() = new_source;

        Ok(())
    }
}

/// [`ChiselCommand`] implementations.
impl ChiselDispatcher {
    /// Dispatches a [`ChiselCommand`].
    pub async fn dispatch_command(&mut self, cmd: ChiselCommand) -> Result<ControlFlow<()>> {
        match cmd {
            ChiselCommand::Quit => Ok(ControlFlow::Break(())),
            cmd => self.dispatch_command_impl(cmd).await.map(ControlFlow::Continue),
        }
    }

    async fn dispatch_command_impl(&mut self, cmd: ChiselCommand) -> Result<()> {
        match cmd {
            ChiselCommand::Help => self.show_help(),
            ChiselCommand::Quit => unreachable!(),
            ChiselCommand::Clear => self.clear_source(),
            ChiselCommand::Save { id } => self.save_session(id),
            ChiselCommand::Load { id } => self.load_session(&id),
            ChiselCommand::ListSessions => self.list_sessions(),
            ChiselCommand::Source => self.show_source(),
            ChiselCommand::ClearCache => self.clear_cache(),
            ChiselCommand::Fork { url } => self.set_fork(url),
            ChiselCommand::Traces => self.toggle_traces(),
            ChiselCommand::Calldata { data } => self.set_calldata(data.as_deref()),
            ChiselCommand::MemDump => self.show_mem_dump().await,
            ChiselCommand::StackDump => self.show_stack_dump().await,
            ChiselCommand::Export => self.export(),
            ChiselCommand::Fetch { addr, name } => self.fetch_interface(addr, name).await,
            ChiselCommand::Exec { command, args } => self.exec_command(command, args),
            ChiselCommand::Edit => self.edit_session().await,
            ChiselCommand::RawStack { var } => self.show_raw_stack(var).await,
        }
    }

    pub(crate) fn show_help(&self) -> Result<()> {
        sh_println!("{}", ChiselCommand::format_help())
    }

    pub(crate) fn clear_source(&mut self) -> Result<()> {
        self.source_mut().clear();
        sh_println!("Cleared session!")
    }

    pub(crate) fn save_session(&mut self, id: Option<String>) -> Result<()> {
        // If a new name was supplied, overwrite the ID of the current session.
        if let Some(id) = id {
            // TODO: Should we delete the old cache file if the id of the session changes?
            self.session.id = Some(id);
        }

        self.session.write()?;
        sh_println!("Saved session to cache with ID = {}", self.session.id.as_ref().unwrap())
    }

    pub(crate) fn load_session(&mut self, id: &str) -> Result<()> {
        // Try to save the current session before loading another.
        // Don't save an empty session.
        if !self.source().run_code.is_empty() {
            self.session.write()?;
            sh_println!("{}", "Saved current session!".green())?;
        }

        let new_session = match id {
            "latest" => ChiselSession::latest(),
            id => ChiselSession::load(id),
        }
        .wrap_err("failed to load session")?;

        new_session.source.build()?;
        self.session = new_session;
        sh_println!("Loaded Chisel session! (ID = {})", self.session.id.as_ref().unwrap())
    }

    pub(crate) fn list_sessions(&self) -> Result<()> {
        let sessions = ChiselSession::get_sessions()?;
        if sessions.is_empty() {
            eyre::bail!("No sessions found. Use the `!save` command to save a session.");
        }
        sh_println!(
            "{}\n{}",
            format!("{CHISEL_CHAR} Chisel Sessions").cyan(),
            sessions
                .iter()
                .map(|(time, name)| format!("{} - {}", format!("{time:?}").blue(), name))
                .collect::<Vec<String>>()
                .join("\n")
        )
    }

    pub(crate) fn show_source(&self) -> Result<()> {
        let formatted = self.format_source().wrap_err("failed to format session source")?;
        let highlighted = self.helper.highlight(&formatted);
        sh_println!("{highlighted}")
    }

    pub(crate) fn clear_cache(&mut self) -> Result<()> {
        ChiselSession::clear_cache().wrap_err("failed to clear cache")?;
        self.session.id = None;
        sh_println!("Cleared chisel cache!")
    }

    pub(crate) fn set_fork(&mut self, url: Option<String>) -> Result<()> {
        let Some(url) = url else {
            self.source_mut().config.evm_opts.fork_url = None;
            sh_println!("Now using local environment.")?;
            return Ok(());
        };

        // If the argument is an RPC alias designated in the
        // `[rpc_endpoints]` section of the `foundry.toml` within
        // the pwd, use the URL matched to the key.
        let endpoint = if let Some(endpoint) =
            self.source_mut().config.foundry_config.rpc_endpoints.get(&url)
        {
            endpoint.clone()
        } else {
            RpcEndpointUrl::Env(url).into()
        };
        let fork_url = endpoint.resolve().url()?;

        if let Err(e) = Url::parse(&fork_url) {
            eyre::bail!("invalid fork URL: {e}");
        }

        sh_println!("Set fork URL to {}", fork_url.yellow())?;

        self.source_mut().config.evm_opts.fork_url = Some(fork_url);
        // Clear the backend so that it is re-instantiated with the new fork
        // upon the next execution of the session source.
        self.source_mut().config.backend = None;

        Ok(())
    }

    pub(crate) fn toggle_traces(&mut self) -> Result<()> {
        let t = &mut self.source_mut().config.traces;
        *t = !*t;
        sh_println!("{} traces!", if *t { "Enabled" } else { "Disabled" })
    }

    pub(crate) fn set_calldata(&mut self, data: Option<&str>) -> Result<()> {
        // remove empty space, double quotes, and 0x prefix
        let arg = data
            .map(|s| s.trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\''))
            .map(|s| s.strip_prefix("0x").unwrap_or(s))
            .unwrap_or("");

        if arg.is_empty() {
            self.source_mut().config.calldata = None;
            sh_println!("Calldata cleared.")?;
            return Ok(());
        }

        let calldata = hex::decode(arg);
        match calldata {
            Ok(calldata) => {
                self.source_mut().config.calldata = Some(calldata);
                sh_println!("Set calldata to '{}'", arg.yellow())
            }
            Err(e) => {
                eyre::bail!("Invalid calldata: {e}")
            }
        }
    }

    pub(crate) async fn show_mem_dump(&mut self) -> Result<()> {
        let (_, res) = self.source_mut().execute().await?;
        let Some((_, mem)) = res.state.as_ref() else {
            eyre::bail!("Run function is empty.");
        };
        for i in (0..mem.len()).step_by(32) {
            let _ = sh_println!(
                "{}: {}",
                format!("[0x{:02x}:0x{:02x}]", i, i + 32).yellow(),
                hex::encode_prefixed(&mem[i..i + 32]).cyan()
            );
        }
        Ok(())
    }

    pub(crate) async fn show_stack_dump(&mut self) -> Result<()> {
        let (_, res) = self.source_mut().execute().await?;
        let Some((stack, _)) = res.state.as_ref() else {
            eyre::bail!("Run function is empty.");
        };
        for i in (0..stack.len()).rev() {
            let _ = sh_println!(
                "{}: {}",
                format!("[{}]", stack.len() - i - 1).yellow(),
                format!("0x{:02x}", stack[i]).cyan()
            );
        }
        Ok(())
    }

    pub(crate) fn export(&self) -> Result<()> {
        // Check if the pwd is a foundry project
        if !Path::new("foundry.toml").exists() {
            eyre::bail!("Must be in a foundry project to export source to script.");
        }

        // Create "script" dir if it does not already exist.
        if !Path::new("script").exists() {
            std::fs::create_dir_all("script")?;
        }

        let formatted_source = self.format_source()?;
        std::fs::write(PathBuf::from("script/REPL.s.sol"), formatted_source)?;
        sh_println!("Exported session source to script/REPL.s.sol!")
    }

    /// Fetches an interface from Etherscan
    pub(crate) async fn fetch_interface(&mut self, address: Address, name: String) -> Result<()> {
        let abis = fetch_abi_from_etherscan(address, &self.source().config.foundry_config)
            .await
            .wrap_err("Failed to fetch ABI from Etherscan")?;
        let (abi, _) = abis
            .into_iter()
            .next()
            .ok_or_else(|| eyre::eyre!("No ABI found for address {address} on Etherscan"))?;
        let code = forge_fmt::format(&abi.to_sol(&name, None), FormatterConfig::default())
            .into_result()?;
        self.source_mut().add_global_code(&code);
        sh_println!("Added {address}'s interface to source as `{name}`")
    }

    pub(crate) fn exec_command(&self, command: String, args: Vec<String>) -> Result<()> {
        let mut cmd = Command::new(command);
        cmd.args(args);
        let _ = cmd.status()?;
        Ok(())
    }

    pub(crate) async fn edit_session(&mut self) -> Result<()> {
        // create a temp file with the content of the run code
        let mut tmp = Builder::new()
            .prefix("chisel-")
            .suffix(".sol")
            .tempfile()
            .wrap_err("Could not create temporary file")?;
        tmp.as_file_mut()
            .write_all(self.source().run_code.as_bytes())
            .wrap_err("Could not write to temporary file")?;

        // open the temp file with the editor
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
        let mut cmd = Command::new(editor);
        cmd.arg(tmp.path());
        let st = cmd.status()?;
        if !st.success() {
            eyre::bail!("Editor exited with {st}");
        }

        let edited_code = std::fs::read_to_string(tmp.path())?;
        let mut new_source = self.source().clone();
        new_source.clear_run();
        new_source.add_run_code(&edited_code);

        // if the editor exited successfully, try to compile the new code
        self.execute_and_replace(new_source).await?;
        sh_println!("Successfully edited `run()` function's body!")
    }

    pub(crate) async fn show_raw_stack(&mut self, var: String) -> Result<()> {
        let source = self.source_mut();
        let line = format!("bytes32 __raw__; assembly {{ __raw__ := {var} }}");
        if let Ok((new_source, _)) = source.clone_with_new_line(line)
            && let (_, Some(res)) = new_source.inspect("__raw__").await?
        {
            sh_println!("{res}")?;
            return Ok(());
        }

        eyre::bail!("Variable must exist within `run()` function.")
    }
}

/// Preprocesses addresses to ensure they are correctly checksummed and returns whether the input
/// only contained trivia (comments, whitespace).
fn preprocess(input: &str) -> (bool, Cow<'_, str>) {
    let mut only_trivia = true;
    let mut new_input = Cow::Borrowed(input);
    for (pos, token) in solar::parse::Cursor::new(input).with_position() {
        use RawTokenKind::*;

        if matches!(token.kind, Whitespace | LineComment { .. } | BlockComment { .. }) {
            continue;
        }
        only_trivia = false;

        // Ensure that addresses are correctly checksummed.
        if let Literal { kind: RawLiteralKind::Int { base: Base::Hexadecimal, .. } } = token.kind
            && token.len == 42
        {
            let range = pos..pos + 42;
            if let Ok(addr) = input[range.clone()].parse::<Address>() {
                new_input.to_mut().replace_range(range, addr.to_checksum_buffer(None).as_str());
            }
        }
    }
    (only_trivia, new_input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trivia() {
        fn only_trivia(s: &str) -> bool {
            let (only_trivia, _new_input) = preprocess(s);
            only_trivia
        }
        assert!(only_trivia("// line comment"));
        assert!(only_trivia("  \n// line \tcomment\n"));
        assert!(!only_trivia("// line \ncomment"));

        assert!(only_trivia("/* block comment */"));
        assert!(only_trivia(" \t\n  /* block \n \t comment */\n"));
        assert!(!only_trivia("/* block \n \t comment */\nwith \tother"));
    }
}
