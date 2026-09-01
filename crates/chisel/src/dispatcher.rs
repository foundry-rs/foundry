//! Dispatcher
//!
//! This module contains the `ChiselDispatcher` struct, which handles the dispatching
//! of both builtin commands and Solidity snippets.

use crate::{
    executor::InspectResult,
    prelude::{ChiselCommand, ChiselResult, ChiselSession, SessionSourceConfig, SolidityHelper},
    source::SessionSource,
};
use alloy_primitives::{Address, hex};
use eyre::{Context, Result};
use forge_fmt::FormatterConfig;
use foundry_cli::utils::fetch_abi_from_etherscan;
use foundry_config::{Chain, Config, RpcEndpointUrl};
use foundry_evm::{
    core::evm::FoundryEvmNetwork,
    decode::decode_console_logs,
    traces::{
        CallTraceDecoder, CallTraceDecoderBuilder, TraceKind, decode_trace_arena,
        identifier::{SignaturesIdentifier, TraceIdentifiers},
        render_trace_arena,
    },
};
use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
use reqwest::Url;
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
pub struct ChiselDispatcher<FEN: FoundryEvmNetwork> {
    pub session: ChiselSession<FEN>,
    pub helper: SolidityHelper,
    last_result: Option<String>,
}

/// Helper function that formats solidity source with the given [FormatterConfig]
pub fn format_source(source: &str, config: FormatterConfig) -> eyre::Result<String> {
    let formatted = forge_fmt::format(source, config).into_result()?;
    Ok(formatted)
}

impl<FEN: FoundryEvmNetwork> ChiselDispatcher<FEN> {
    /// Associated public function to create a new Dispatcher instance
    pub fn new(config: SessionSourceConfig<FEN>) -> eyre::Result<Self> {
        let session = ChiselSession::new(config)?;
        Ok(Self { session, helper: Default::default(), last_result: None })
    }

    /// Returns the optional ID of the current session.
    pub fn id(&self) -> Option<&str> {
        self.session.id.as_deref()
    }

    /// Returns the [`SessionSource`].
    pub const fn source(&self) -> &SessionSource<FEN> {
        &self.session.source
    }

    /// Returns the [`SessionSource`].
    pub const fn source_mut(&mut self) -> &mut SessionSource<FEN> {
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
    pub async fn dispatch(&mut self, input: &str) -> Result<ControlFlow<()>> {
        if let Some(command) = input.strip_prefix(COMMAND_LEADER) {
            return match ChiselCommand::parse(command) {
                Ok(cmd) => self.dispatch_command(cmd).await,
                Err(e) => {
                    eyre::bail!("unrecognized command: {e}");
                }
            };
        }

        self.dispatch_solidity(input).await
    }

    /// Dispatches an input as Solidity without interpreting Chisel commands.
    pub(crate) async fn dispatch_solidity(&mut self, mut input: &str) -> Result<ControlFlow<()>> {
        input = input.trim();
        let (only_trivia, new_input) = preprocess(input, self.last_result.as_deref())?;
        input = &*new_input;

        let source = self.source_mut();

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

        let InspectResult { control_flow, formatted_output, last_result, replay_input } =
            source.inspect(input).await?;
        let (new_source, do_execute) = if let Some(input) = replay_input {
            source.clone_with_new_line(input)?
        } else {
            (new_source, do_execute)
        };
        if let Some(last_result) = last_result {
            self.last_result = Some(last_result);
        }
        if let Some(res) = &formatted_output {
            let _ = sh_println!("{res}");
        }
        if control_flow.is_break() {
            debug!(%input, ?formatted_output, "inspect success");
            return Ok(ControlFlow::Continue(()));
        }

        if do_execute {
            self.execute_and_replace(new_source).await?;
        } else {
            let out = new_source.build()?;
            debug!(%input, ?out, "skipped execute and rebuild source");
            *self.source_mut() = new_source;
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Decodes traces in the given [`ChiselResult`].
    // TODO: Add `known_contracts` back in.
    pub async fn decode_traces(
        session_config: &SessionSourceConfig<FEN>,
        result: &mut ChiselResult,
        // known_contracts: &ContractsByArtifact,
    ) -> eyre::Result<CallTraceDecoder> {
        let chain_id = session_config.source_chain_id.map(Chain::from);
        let resolved_hardfork = session_config.resolved_hardfork;

        let builder = CallTraceDecoderBuilder::new()
            .with_labels(result.labeled_addresses.clone())
            .with_signature_identifier(SignaturesIdentifier::from_config(
                &session_config.foundry_config,
            )?)
            .with_networks(session_config.foundry_config.networks)
            .with_chain_id(chain_id.map(|c| c.id()))
            .with_hardfork(resolved_hardfork);
        let mut decoder = builder.build();

        let mut identifier =
            TraceIdentifiers::new().with_external(&session_config.foundry_config, chain_id)?;
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

    async fn execute_and_replace(&mut self, mut new_source: SessionSource<FEN>) -> Result<()> {
        let mut res = new_source.execute().await?;
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
impl<FEN: FoundryEvmNetwork> ChiselDispatcher<FEN> {
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
            ChiselCommand::Fork { url } => self.set_fork(url).await,
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
        self.last_result = None;
        sh_println!("Cleared session!")
    }

    pub(crate) fn save_session(&mut self, id: Option<String>) -> Result<()> {
        let previous_id = self.session.id.clone();

        // If a new name was supplied, overwrite the ID of the current session.
        if let Some(id) = id {
            self.session.id = Some(id);
        }

        let new_cache_file = match self.session.write() {
            Ok(path) => path,
            Err(error) => {
                self.session.id = previous_id;
                return Err(error);
            }
        };

        if let (Some(previous_id), Some(current_id)) = (previous_id, self.session.id.as_deref())
            && previous_id != current_id
        {
            let old_cache_file =
                format!("{}chisel-{previous_id}.json", ChiselSession::<FEN>::cache_dir()?);
            let same_cache_file = std::fs::canonicalize(&old_cache_file).ok()
                == std::fs::canonicalize(&new_cache_file).ok();
            if !same_cache_file {
                ChiselSession::<FEN>::remove_cached_session(&previous_id)?;
            }
        }

        sh_println!("Saved session to cache with ID = {}", self.session.id.as_ref().unwrap())
    }

    pub(crate) fn load_session(&mut self, id: &str) -> Result<()> {
        // Try to save the current session before loading another.
        // Don't save an empty session.
        if !self.source().run_code.is_empty() {
            self.session.write()?;
            sh_println!("{}", "Saved current session!".green())?;
        }

        let executor_builder = self.session.source.config.executor_builder.clone();
        let mut new_session = match id {
            "latest" => ChiselSession::<FEN>::latest(executor_builder),
            id => ChiselSession::<FEN>::load(id, executor_builder),
        }
        .wrap_err("failed to load session")?;

        ensure_loaded_session_network_matches(
            &self.session.source.config.foundry_config,
            &new_session.source.config.foundry_config,
            id,
        )?;
        new_session.source.config.foundry_config.force =
            self.session.source.config.foundry_config.force;
        new_session.source.config.initialize_local_context();
        new_session.source.build()?;
        self.session = new_session;
        self.last_result = None;
        sh_println!("Loaded Chisel session! (ID = {})", self.session.id.as_ref().unwrap())
    }

    pub(crate) fn list_sessions(&self) -> Result<()> {
        let sessions = ChiselSession::<FEN>::get_sessions()?;
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
        ChiselSession::<FEN>::clear_cache().wrap_err("failed to clear cache")?;
        self.session.id = None;
        sh_println!("Cleared chisel cache!")
    }

    pub(crate) async fn set_fork(&mut self, url: Option<String>) -> Result<()> {
        self.source_mut().config.initialize_local_context();

        let Some(url) = url else {
            return self.clear_fork();
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

        let mut fork_opts = self.source().config.evm_opts.clone();
        fork_opts.fork_url = Some(fork_url.clone());
        fork_opts.fork_block_number = None;
        fork_opts.fork_block_number_is_inferred = false;
        let explicit_network =
            fork_opts.networks.has_network_selection() && !fork_opts.fork_network_is_inferred;
        let identity = fork_opts.discover_fork_endpoint().await?;
        let target = identity.network;
        let current_opts = &self.source().config.evm_opts;
        let current = network_variant(current_opts.networks);
        ensure_fork_network_matches(current, target)?;

        let networks = if explicit_network {
            current_opts.networks
        } else {
            current_opts.networks.with_rpc_profile(identity.network_profile)
        };
        if fork_opts.env.chain_id.is_none() || fork_opts.fork_chain_id_is_inferred {
            fork_opts.env.chain_id = Some(identity.execution_chain_id);
            fork_opts.fork_chain_id_is_inferred = true;
        }
        fork_opts.networks = networks;
        fork_opts.fork_endpoint = Some(identity.clone());
        fork_opts.fork_network_is_inferred = !explicit_network;
        fork_opts.pin_fork_block().await?;
        let chain_id_is_inferred = fork_opts.fork_chain_id_is_inferred;
        let source = self.source_mut();
        source.config.evm_opts = fork_opts;
        source.config.fork_network_is_inferred = !explicit_network;
        source.config.fork_chain_id_is_inferred = chain_id_is_inferred;
        source.config.foundry_config.networks = networks;
        source.config.foundry_config.chain = Some(Chain::from(identity.source_chain_id));
        source.config.resolved_hardfork = None;
        source.config.source_chain_id = None;
        // Clear the backend so that it is re-instantiated with the new fork
        // upon the next execution of the session source.
        source.config.cached_backend = None;

        sh_println!("Set fork URL to {}", fork_url.yellow())?;

        Ok(())
    }

    fn clear_fork(&mut self) -> Result<()> {
        let current = network_variant(self.source().config.evm_opts.networks);
        let local_networks =
            self.source().config.local_networks.unwrap_or(self.source().config.evm_opts.networks);
        let target = network_variant(local_networks);
        ensure_fork_network_matches(current, target)?;

        let source = self.source_mut();
        source.config.evm_opts.fork_url = None;
        source.config.evm_opts.fork_block_number = None;
        source.config.evm_opts.fork_block_number_is_inferred = false;
        source.config.evm_opts.networks = local_networks;
        source.config.evm_opts.env.chain_id = source.config.local_chain_id;
        source.config.evm_opts.fork_network_is_inferred = false;
        source.config.evm_opts.fork_chain_id_is_inferred = false;
        source.config.fork_network_is_inferred = false;
        source.config.fork_chain_id_is_inferred = false;
        source.config.foundry_config.networks = local_networks;
        source.config.foundry_config.chain = source.config.local_chain_id.map(Chain::from);
        source.config.resolved_hardfork = None;
        source.config.source_chain_id = None;
        source.config.cached_backend = None;
        sh_println!("Now using local environment.")
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
                eyre::bail!("Invalid calldata: {e}");
            }
        }
    }

    pub(crate) async fn show_mem_dump(&mut self) -> Result<()> {
        let res = self.source_mut().execute().await?;
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
        let res = self.source_mut().execute().await?;
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
            && let InspectResult { formatted_output: Some(res), .. } =
                new_source.inspect("__raw__").await?
        {
            sh_println!("{res}")?;
            return Ok(());
        }

        eyre::bail!("Variable must exist within `run()` function.");
    }
}

fn config_network_name(config: &Config) -> &'static str {
    config.networks.active_network_name().unwrap_or("ethereum")
}

fn network_variant(networks: NetworkConfigs) -> NetworkVariant {
    networks.resolved_network().unwrap_or_default()
}

fn ensure_fork_network_matches(current: NetworkVariant, target: NetworkVariant) -> Result<()> {
    if current != target {
        eyre::bail!(
            "cannot switch this Chisel session from network `{current}` to `{target}`. Restart \
             Chisel with `--network {target}` or a fork URL for that network.",
        );
    }
    Ok(())
}

fn ensure_loaded_session_network_matches(
    current: &Config,
    loaded: &Config,
    id: &str,
) -> Result<()> {
    let current_network = config_network_name(current);
    let loaded_network = config_network_name(loaded);
    if current_network != loaded_network {
        eyre::bail!(
            "Chisel session `{id}` was saved for network `{loaded_network}`, but the current \
             network is `{current_network}`. Rerun with `--network {loaded_network}` to load it.",
        );
    }
    Ok(())
}

/// Expands the previous result, checksums addresses, and returns whether the input only contained
/// trivia (comments, whitespace).
fn preprocess<'a>(input: &'a str, last_result: Option<&str>) -> Result<(bool, Cow<'a, str>)> {
    let mut only_trivia = true;
    let mut replacements = Vec::new();
    for (pos, token) in solar::parse::Cursor::new(input).with_position() {
        use RawTokenKind::{BlockComment, LineComment, Literal, Whitespace};

        if matches!(token.kind, Whitespace | LineComment { .. } | BlockComment { .. }) {
            continue;
        }
        only_trivia = false;

        let range = pos..pos + token.len as usize;
        if &input[range.clone()] == "$_" {
            let last_result = last_result.ok_or_else(|| eyre::eyre!("no previous result"))?;
            replacements.push((range, format!("({last_result})")));
            continue;
        }

        // Ensure that addresses are correctly checksummed.
        if let Literal { kind: RawLiteralKind::Int { base: Base::Hexadecimal, .. } } = token.kind
            && token.len == 42
            && let Ok(addr) = input[range.clone()].parse::<Address>()
        {
            replacements.push((range, addr.to_checksum_buffer(None).to_string()));
        }
    }

    if replacements.is_empty() {
        Ok((only_trivia, Cow::Borrowed(input)))
    } else {
        let mut new_input = input.to_string();
        for (range, replacement) in replacements.into_iter().rev() {
            new_input.replace_range(range, &replacement);
        }
        Ok((only_trivia, Cow::Owned(new_input)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm::{core::evm::EthEvmNetwork, opts::EvmOpts};

    fn config_with_network(network: Option<&str>) -> Config {
        let mut config = Config::default();
        if let Some(network) = network {
            config.networks = serde_json::from_value(serde_json::json!({
                "network": network,
                "celo": false,
                "bypass_prevrandao": false,
            }))
            .unwrap();
        }
        config
    }

    #[test]
    fn config_network_name_defaults_to_ethereum() {
        assert_eq!(config_network_name(&Config::default()), "ethereum");
    }

    #[test]
    fn ensure_fork_network_matches_accepts_same_family() {
        ensure_fork_network_matches(NetworkVariant::Ethereum, NetworkVariant::Ethereum).unwrap();
        ensure_fork_network_matches(NetworkVariant::Tempo, NetworkVariant::Tempo).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn setting_fork_preserves_explicit_celo_context() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let networks = NetworkConfigs::with_celo();
        let config = SessionSourceConfig::<EthEvmNetwork> {
            foundry_config: Config { networks, ..Default::default() },
            evm_opts: EvmOpts { networks, ..Default::default() },
            local_networks: Some(networks),
            ..Default::default()
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();

        dispatcher.set_fork(Some(handle.http_endpoint())).await.unwrap();
        assert!(dispatcher.source().config.evm_opts.networks.is_celo());
        assert!(!dispatcher.source().config.evm_opts.fork_network_is_inferred);

        dispatcher.clear_fork().unwrap();
        assert!(dispatcher.source().config.evm_opts.networks.is_celo());
    }

    #[test]
    #[cfg(feature = "monad")]
    fn ensure_fork_network_matches_rejects_cross_family_change() {
        let err = ensure_fork_network_matches(NetworkVariant::Ethereum, NetworkVariant::Monad)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot switch this Chisel session from network `ethereum` to `monad`. Restart Chisel \
             with `--network monad` or a fork URL for that network."
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn clearing_startup_fork_preserves_inferred_monad_context() {
        let networks = NetworkConfigs::with_monad();
        let evm_opts = EvmOpts {
            fork_url: Some("http://localhost:8545".to_string()),
            networks,
            env: foundry_evm::opts::Env { chain_id: Some(143), ..Default::default() },
            ..Default::default()
        };
        let config = SessionSourceConfig::<foundry_evm::core::evm::MonadEvmNetwork> {
            foundry_config: Config {
                solc: Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 29))),
                networks,
                chain: Some(Chain::from(143u64)),
                ..Default::default()
            },
            evm_opts,
            local_networks: Some(networks),
            local_chain_id: Some(143),
            ..Default::default()
        };
        let mut dispatcher = ChiselDispatcher::new(config).unwrap();

        dispatcher.clear_fork().unwrap();

        let config = &dispatcher.source().config;
        assert!(config.evm_opts.fork_url.is_none());
        assert!(config.evm_opts.networks.is_monad());
        assert_eq!(config.evm_opts.env.chain_id, Some(143));
        assert!(config.foundry_config.networks.is_monad());
        assert_eq!(config.foundry_config.chain.map(|chain| chain.id()), Some(143));
    }

    #[test]
    fn ensure_loaded_session_network_matches_rejects_different_network() {
        let current = config_with_network(None);
        let loaded = config_with_network(Some("tempo"));

        let err = ensure_loaded_session_network_matches(&current, &loaded, "42").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Chisel session `42` was saved for network `tempo`, but the current network is \
             `ethereum`. Rerun with `--network tempo` to load it."
        );
    }

    #[test]
    #[cfg(feature = "monad")]
    fn ensure_loaded_session_network_matches_rejects_monad_on_default_network() {
        let current = config_with_network(None);
        let loaded = config_with_network(Some("monad"));

        let err = ensure_loaded_session_network_matches(&current, &loaded, "43").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Chisel session `43` was saved for network `monad`, but the current network is \
             `ethereum`. Rerun with `--network monad` to load it."
        );
    }

    #[test]
    fn ensure_loaded_session_network_matches_accepts_same_network() {
        let current = config_with_network(Some("tempo"));
        let loaded = config_with_network(Some("tempo"));

        ensure_loaded_session_network_matches(&current, &loaded, "42").unwrap();
    }

    #[cfg(feature = "base")]
    #[test]
    fn ensure_loaded_session_network_matches_preserves_base() {
        let base = config_with_network(Some("base"));
        ensure_loaded_session_network_matches(&base, &base, "42").unwrap();

        let err =
            ensure_loaded_session_network_matches(&Config::default(), &base, "42").unwrap_err();
        assert!(err.to_string().contains("Rerun with `--network base`"), "{err}");
    }

    #[test]
    fn test_trivia() {
        fn only_trivia(s: &str) -> bool {
            let (only_trivia, _new_input) = preprocess(s, None).unwrap();
            only_trivia
        }
        assert!(only_trivia("// line comment"));
        assert!(only_trivia("  \n// line \tcomment\n"));
        assert!(!only_trivia("// line \ncomment"));

        assert!(only_trivia("/* block comment */"));
        assert!(only_trivia(" \t\n  /* block \n \t comment */\n"));
        assert!(!only_trivia("/* block \n \t comment */\nwith \tother"));
    }

    #[test]
    fn test_last_result_preprocessing() {
        let result = "abi.decode(hex\"2a\", (uint256))";
        let (_, input) = preprocess("uint256 answer = $_;", Some(result)).unwrap();
        assert_eq!(input, format!("uint256 answer = ({result});"));

        let literal = r#"string memory value = "$_"; // $_"#;
        let (_, input) = preprocess(literal, Some(result)).unwrap();
        assert_eq!(input, literal);

        assert_eq!(preprocess("$_", None).unwrap_err().to_string(), "no previous result");
    }
}
