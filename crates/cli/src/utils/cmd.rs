use alloy_json_abi::JsonAbi;
use alloy_primitives::Address;
use eyre::{Result, WrapErr};
use foundry_common::{compile::ProjectCompiler, fs, shell, ContractsByArtifact, TestFunctionExt};
use foundry_compilers::{
    artifacts::{CompactBytecode, Settings},
    cache::{CacheEntry, CompilerCache},
    utils::read_json_file,
    Artifact, ArtifactId, ProjectCompileOutput,
};
use foundry_config::{error::ExtractConfigError, figment::Figment, Chain, Config, NamedChain};
use foundry_debugger::Debugger;
use foundry_evm::{
    executors::{DeployResult, EvmError, RawCallResult},
    opts::EvmOpts,
    traces::{
        debug::{ContractSources, DebugTraceIdentifier},
        decode_trace_arena,
        identifier::{CachedSignatures, SignaturesIdentifier, TraceIdentifiers},
        render_trace_arena_inner, CallTraceDecoder, CallTraceDecoderBuilder, TraceKind, Traces,
    },
};
use std::{
    fmt::Write,
    path::{Path, PathBuf},
    str::FromStr,
};
use yansi::Paint;

/// Given a `Project`'s output, removes the matching ABI, Bytecode and
/// Runtime Bytecode of the given contract.
#[track_caller]
pub fn remove_contract(
    output: ProjectCompileOutput,
    path: &Path,
    name: &str,
) -> Result<(JsonAbi, CompactBytecode, ArtifactId)> {
    let mut other = Vec::new();
    let Some((id, contract)) = output.into_artifacts().find_map(|(id, artifact)| {
        if id.name == name && id.source == path {
            Some((id, artifact))
        } else {
            other.push(id.name);
            None
        }
    }) else {
        let mut err = format!("could not find artifact: `{name}`");
        if let Some(suggestion) = super::did_you_mean(name, other).pop() {
            if suggestion != name {
                err = format!(
                    r#"{err}

        Did you mean `{suggestion}`?"#
                );
            }
        }
        eyre::bail!(err)
    };

    let abi = contract
        .get_abi()
        .ok_or_else(|| eyre::eyre!("contract {} does not contain abi", name))?
        .into_owned();

    let bin = contract
        .get_bytecode()
        .ok_or_else(|| eyre::eyre!("contract {} does not contain bytecode", name))?
        .into_owned();

    Ok((abi, bin, id))
}

/// Helper function for finding a contract by ContractName
// TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
// contract name?
pub fn get_cached_entry_by_name(
    cache: &CompilerCache<Settings>,
    name: &str,
) -> Result<(PathBuf, CacheEntry)> {
    let mut cached_entry = None;
    let mut alternatives = Vec::new();

    for (abs_path, entry) in cache.files.iter() {
        for (artifact_name, _) in entry.artifacts.iter() {
            if artifact_name == name {
                if cached_entry.is_some() {
                    eyre::bail!(
                        "contract with duplicate name `{}`. please pass the path instead",
                        name
                    )
                }
                cached_entry = Some((abs_path.to_owned(), entry.to_owned()));
            } else {
                alternatives.push(artifact_name);
            }
        }
    }

    if let Some(entry) = cached_entry {
        return Ok(entry);
    }

    let mut err = format!("could not find artifact: `{name}`");
    if let Some(suggestion) = super::did_you_mean(name, &alternatives).pop() {
        err = format!(
            r#"{err}

        Did you mean `{suggestion}`?"#
        );
    }
    eyre::bail!(err)
}

/// Returns error if constructor has arguments.
pub fn ensure_clean_constructor(abi: &JsonAbi) -> Result<()> {
    if let Some(constructor) = &abi.constructor {
        if !constructor.inputs.is_empty() {
            eyre::bail!("Contract constructor should have no arguments. Add those arguments to  `run(...)` instead, and call it with `--sig run(...)`.");
        }
    }
    Ok(())
}

pub fn needs_setup(abi: &JsonAbi) -> bool {
    let setup_fns: Vec<_> = abi.functions().filter(|func| func.name.is_setup()).collect();

    for setup_fn in setup_fns.iter() {
        if setup_fn.name != "setUp" {
            let _ = sh_warn!(
                "Found invalid setup function \"{}\" did you mean \"setUp()\"?",
                setup_fn.signature()
            );
        }
    }

    setup_fns.len() == 1 && setup_fns[0].name == "setUp"
}

pub fn eta_key(state: &indicatif::ProgressState, f: &mut dyn Write) {
    write!(f, "{:.1}s", state.eta().as_secs_f64()).unwrap()
}

pub fn init_progress(len: u64, label: &str) -> indicatif::ProgressBar {
    let pb = indicatif::ProgressBar::new(len);
    let mut template =
        "{prefix}{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} "
            .to_string();
    write!(template, "{label}").unwrap();
    template += " ({eta})";
    pb.set_style(
        indicatif::ProgressStyle::with_template(&template)
            .unwrap()
            .with_key("eta", crate::utils::eta_key)
            .progress_chars("#>-"),
    );
    pb
}

/// True if the network calculates gas costs differently.
pub fn has_different_gas_calc(chain_id: u64) -> bool {
    if let Some(chain) = Chain::from(chain_id).named() {
        return chain.is_arbitrum() ||
            matches!(
                chain,
                NamedChain::Acala |
                    NamedChain::AcalaMandalaTestnet |
                    NamedChain::AcalaTestnet |
                    NamedChain::Etherlink |
                    NamedChain::EtherlinkTestnet |
                    NamedChain::Karura |
                    NamedChain::KaruraTestnet |
                    NamedChain::Mantle |
                    NamedChain::MantleSepolia |
                    NamedChain::MantleTestnet |
                    NamedChain::Moonbase |
                    NamedChain::Moonbeam |
                    NamedChain::MoonbeamDev |
                    NamedChain::Moonriver
            );
    }
    false
}

/// True if it supports broadcasting in batches.
pub fn has_batch_support(chain_id: u64) -> bool {
    if let Some(chain) = Chain::from(chain_id).named() {
        return !chain.is_arbitrum();
    }
    true
}

/// Helpers for loading configuration.
///
/// This is usually implicitly implemented on a "&CmdArgs" struct via impl macros defined in
/// `forge_config` (see [`foundry_config::impl_figment_convert`] for more details) and the impl
/// definition on `T: Into<Config> + Into<Figment>` below.
///
/// Each function also has an `emit_warnings` form which does the same thing as its counterpart but
/// also prints `Config::__warnings` to stderr
pub trait LoadConfig {
    /// Load and sanitize the [`Config`] based on the options provided in self
    ///
    /// Returns an error if loading the config failed
    fn try_load_config(self) -> Result<Config, ExtractConfigError>;
    /// Load and sanitize the [`Config`] based on the options provided in self
    fn load_config(self) -> Config;
    /// Load and sanitize the [`Config`], as well as extract [`EvmOpts`] from self
    fn load_config_and_evm_opts(self) -> Result<(Config, EvmOpts)>;
    /// Load [`Config`] but do not sanitize. See [`Config::sanitized`] for more information
    fn load_config_unsanitized(self) -> Config;
    /// Load [`Config`] but do not sanitize. See [`Config::sanitized`] for more information.
    ///
    /// Returns an error if loading failed
    fn try_load_config_unsanitized(self) -> Result<Config, ExtractConfigError>;
    /// Same as [`LoadConfig::load_config`] but also emits warnings generated
    fn load_config_emit_warnings(self) -> Config;
    /// Same as [`LoadConfig::load_config`] but also emits warnings generated
    ///
    /// Returns an error if loading failed
    fn try_load_config_emit_warnings(self) -> Result<Config, ExtractConfigError>;
    /// Same as [`LoadConfig::load_config_and_evm_opts`] but also emits warnings generated
    fn load_config_and_evm_opts_emit_warnings(self) -> Result<(Config, EvmOpts)>;
    /// Same as [`LoadConfig::load_config_unsanitized`] but also emits warnings generated
    fn load_config_unsanitized_emit_warnings(self) -> Config;
    fn try_load_config_unsanitized_emit_warnings(self) -> Result<Config, ExtractConfigError>;
}

impl<T> LoadConfig for T
where
    T: Into<Config> + Into<Figment>,
{
    fn try_load_config(self) -> Result<Config, ExtractConfigError> {
        let figment: Figment = self.into();
        Ok(Config::try_from(figment)?.sanitized())
    }

    fn load_config(self) -> Config {
        self.into()
    }

    fn load_config_and_evm_opts(self) -> Result<(Config, EvmOpts)> {
        let figment: Figment = self.into();

        let mut evm_opts = figment.extract::<EvmOpts>().map_err(ExtractConfigError::new)?;
        let config = Config::try_from(figment)?.sanitized();

        // update the fork url if it was an alias
        if let Some(fork_url) = config.get_rpc_url() {
            trace!(target: "forge::config", ?fork_url, "Update EvmOpts fork url");
            evm_opts.fork_url = Some(fork_url?.into_owned());
        }

        Ok((config, evm_opts))
    }

    fn load_config_unsanitized(self) -> Config {
        let figment: Figment = self.into();
        Config::from_provider(figment)
    }

    fn try_load_config_unsanitized(self) -> Result<Config, ExtractConfigError> {
        let figment: Figment = self.into();
        Config::try_from(figment)
    }

    fn load_config_emit_warnings(self) -> Config {
        let config = self.load_config();
        config.warnings.iter().for_each(|w| sh_warn!("{w}").unwrap());
        config
    }

    fn try_load_config_emit_warnings(self) -> Result<Config, ExtractConfigError> {
        let config = self.try_load_config()?;
        emit_warnings(&config);
        Ok(config)
    }

    fn load_config_and_evm_opts_emit_warnings(self) -> Result<(Config, EvmOpts)> {
        let (config, evm_opts) = self.load_config_and_evm_opts()?;
        emit_warnings(&config);
        Ok((config, evm_opts))
    }

    fn load_config_unsanitized_emit_warnings(self) -> Config {
        let config = self.load_config_unsanitized();
        emit_warnings(&config);
        config
    }

    fn try_load_config_unsanitized_emit_warnings(self) -> Result<Config, ExtractConfigError> {
        let config = self.try_load_config_unsanitized()?;
        emit_warnings(&config);
        Ok(config)
    }
}

fn emit_warnings(config: &Config) {
    for warning in &config.warnings {
        let _ = sh_warn!("{warning}");
    }
}

/// Read contract constructor arguments from the given file.
pub fn read_constructor_args_file(constructor_args_path: PathBuf) -> Result<Vec<String>> {
    if !constructor_args_path.exists() {
        eyre::bail!("Constructor args file \"{}\" not found", constructor_args_path.display());
    }
    let args = if constructor_args_path.extension() == Some(std::ffi::OsStr::new("json")) {
        read_json_file(&constructor_args_path).wrap_err(format!(
            "Constructor args file \"{}\" must encode a json array",
            constructor_args_path.display(),
        ))?
    } else {
        fs::read_to_string(constructor_args_path)?.split_whitespace().map(str::to_string).collect()
    };
    Ok(args)
}

/// A slimmed down return from the executor used for returning minimal trace + gas metering info
#[derive(Debug)]
pub struct TraceResult {
    pub success: bool,
    pub traces: Option<Traces>,
    pub gas_used: u64,
}

impl TraceResult {
    /// Create a new [`TraceResult`] from a [`RawCallResult`].
    pub fn from_raw(raw: RawCallResult, trace_kind: TraceKind) -> Self {
        let RawCallResult { gas_used, traces, reverted, .. } = raw;
        Self { success: !reverted, traces: traces.map(|arena| vec![(trace_kind, arena)]), gas_used }
    }
}

impl From<DeployResult> for TraceResult {
    fn from(result: DeployResult) -> Self {
        Self::from_raw(result.raw, TraceKind::Deployment)
    }
}

impl TryFrom<Result<DeployResult, EvmError>> for TraceResult {
    type Error = EvmError;

    fn try_from(value: Result<DeployResult, EvmError>) -> Result<Self, Self::Error> {
        match value {
            Ok(result) => Ok(Self::from(result)),
            Err(EvmError::Execution(err)) => Ok(Self::from_raw(err.raw, TraceKind::Deployment)),
            Err(err) => Err(err),
        }
    }
}

impl From<RawCallResult> for TraceResult {
    fn from(result: RawCallResult) -> Self {
        Self::from_raw(result, TraceKind::Execution)
    }
}

impl TryFrom<Result<RawCallResult>> for TraceResult {
    type Error = EvmError;

    fn try_from(value: Result<RawCallResult>) -> Result<Self, Self::Error> {
        match value {
            Ok(result) => Ok(Self::from(result)),
            Err(err) => Err(EvmError::from(err)),
        }
    }
}

/// labels the traces, conditionally prints them or opens the debugger
pub async fn handle_traces(
    mut result: TraceResult,
    config: &Config,
    chain: Option<Chain>,
    labels: Vec<String>,
    with_local_artifacts: bool,
    debug: bool,
    decode_internal: bool,
) -> Result<()> {
    let (known_contracts, mut sources) = if with_local_artifacts {
        let _ = sh_println!("Compiling project to generate artifacts");
        let project = config.project()?;
        let compiler = ProjectCompiler::new();
        let output = compiler.compile(&project)?;
        (
            Some(ContractsByArtifact::new(
                output.artifact_ids().map(|(id, artifact)| (id, artifact.clone().into())),
            )),
            ContractSources::from_project_output(&output, project.root(), None)?,
        )
    } else {
        (None, ContractSources::default())
    };

    let labels = labels.iter().filter_map(|label_str| {
        let mut iter = label_str.split(':');

        if let Some(addr) = iter.next() {
            if let (Ok(address), Some(label)) = (Address::from_str(addr), iter.next()) {
                return Some((address, label.to_string()));
            }
        }
        None
    });
    let config_labels = config.labels.clone().into_iter();

    let mut builder = CallTraceDecoderBuilder::new()
        .with_labels(labels.chain(config_labels))
        .with_signature_identifier(SignaturesIdentifier::new(
            Config::foundry_cache_dir(),
            config.offline,
        )?);
    let mut identifier = TraceIdentifiers::new().with_etherscan(config, chain)?;
    if let Some(contracts) = &known_contracts {
        builder = builder.with_known_contracts(contracts);
        identifier = identifier.with_local(contracts);
    }

    let mut decoder = builder.build();

    for (_, trace) in result.traces.as_deref_mut().unwrap_or_default() {
        decoder.identify(trace, &mut identifier);
    }

    if decode_internal || debug {
        if let Some(ref etherscan_identifier) = identifier.etherscan {
            sources.merge(etherscan_identifier.get_compiled_contracts().await?);
        }

        if debug {
            let mut debugger = Debugger::builder()
                .traces(result.traces.expect("missing traces"))
                .decoder(&decoder)
                .sources(sources)
                .build();
            debugger.try_run_tui()?;
            return Ok(())
        }

        decoder.debug_identifier = Some(DebugTraceIdentifier::new(sources));
    }

    print_traces(&mut result, &decoder, shell::verbosity() > 0, shell::verbosity() > 4).await?;

    Ok(())
}

pub async fn print_traces(
    result: &mut TraceResult,
    decoder: &CallTraceDecoder,
    verbose: bool,
    state_changes: bool,
) -> Result<()> {
    let traces = result.traces.as_mut().expect("No traces found");

    if !shell::is_json() {
        sh_println!("Traces:")?;
    }

    for (_, arena) in traces {
        decode_trace_arena(arena, decoder).await?;
        sh_println!("{}", render_trace_arena_inner(arena, verbose, state_changes))?;
    }

    if shell::is_json() {
        return Ok(());
    }

    sh_println!()?;
    if result.success {
        sh_println!("{}", "Transaction successfully executed.".green())?;
    } else {
        sh_err!("Transaction failed.")?;
    }
    sh_println!("Gas used: {}", result.gas_used)?;

    Ok(())
}

/// Traverse the artifacts in the project to generate local signatures and merge them into the cache
/// file.
pub fn cache_local_signatures(output: &ProjectCompileOutput, cache_path: PathBuf) -> Result<()> {
    let path = cache_path.join("signatures");
    let mut cached_signatures = CachedSignatures::load(cache_path);
    output.artifacts().for_each(|(_, artifact)| {
        if let Some(abi) = &artifact.abi {
            for func in abi.functions() {
                cached_signatures.functions.insert(func.selector().to_string(), func.signature());
            }
            for event in abi.events() {
                cached_signatures
                    .events
                    .insert(event.selector().to_string(), event.full_signature());
            }
            for error in abi.errors() {
                cached_signatures.errors.insert(error.selector().to_string(), error.signature());
            }
            // External libraries doesn't have functions included in abi, but `methodIdentifiers`.
            if let Some(method_identifiers) = &artifact.method_identifiers {
                method_identifiers.iter().for_each(|(signature, selector)| {
                    cached_signatures
                        .functions
                        .entry(format!("0x{selector}"))
                        .or_insert(signature.to_string());
                });
            }
        }
    });

    fs::write_json_file(&path, &cached_signatures)?;
    Ok(())
}
