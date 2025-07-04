use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, Bytes, map::HashMap};
use eyre::{Result, WrapErr};
use foundry_common::{
    ContractsByArtifact, TestFunctionExt, compile::ProjectCompiler, fs, selectors::SelectorKind,
    shell,
};
use foundry_compilers::{
    Artifact, ArtifactId, ProjectCompileOutput,
    artifacts::{CompactBytecode, Settings},
    cache::{CacheEntry, CompilerCache},
    utils::read_json_file,
};
use foundry_config::{Chain, Config, NamedChain, error::ExtractConfigError, figment::Figment};
use foundry_debugger::Debugger;
use foundry_evm::{
    executors::{DeployResult, EvmError, RawCallResult},
    opts::EvmOpts,
    traces::{
        CallTraceDecoder, CallTraceDecoderBuilder, TraceKind, Traces,
        debug::{ContractSources, DebugTraceIdentifier},
        decode_trace_arena,
        identifier::{SignaturesCache, SignaturesIdentifier, TraceIdentifiers},
        render_trace_arena_inner,
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
        if let Some(suggestion) = super::did_you_mean(name, other).pop()
            && suggestion != name
        {
            err = format!(
                r#"{err}

        Did you mean `{suggestion}`?"#
            );
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

    for (abs_path, entry) in &cache.files {
        for artifact_name in entry.artifacts.keys() {
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
    if let Some(constructor) = &abi.constructor
        && !constructor.inputs.is_empty()
    {
        eyre::bail!(
            "Contract constructor should have no arguments. Add those arguments to  `run(...)` instead, and call it with `--sig run(...)`."
        );
    }
    Ok(())
}

pub fn needs_setup(abi: &JsonAbi) -> bool {
    let setup_fns: Vec<_> = abi.functions().filter(|func| func.name.is_setup()).collect();

    for setup_fn in &setup_fns {
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
        return chain.is_arbitrum()
            || chain.is_elastic()
            || matches!(
                chain,
                NamedChain::Acala
                    | NamedChain::AcalaMandalaTestnet
                    | NamedChain::AcalaTestnet
                    | NamedChain::Etherlink
                    | NamedChain::EtherlinkTestnet
                    | NamedChain::Karura
                    | NamedChain::KaruraTestnet
                    | NamedChain::Mantle
                    | NamedChain::MantleSepolia
                    | NamedChain::MantleTestnet
                    | NamedChain::Moonbase
                    | NamedChain::Moonbeam
                    | NamedChain::MoonbeamDev
                    | NamedChain::Moonriver
                    | NamedChain::Metis
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
/// This is usually implemented through the macros defined in [`foundry_config`]. See
/// [`foundry_config::impl_figment_convert`] for more details.
///
/// By default each function will emit warnings generated during loading, unless the `_no_warnings`
/// variant is used.
pub trait LoadConfig {
    /// Load the [`Config`] based on the options provided in self.
    fn figment(&self) -> Figment;

    /// Load and sanitize the [`Config`] based on the options provided in self.
    fn load_config(&self) -> Result<Config, ExtractConfigError> {
        self.load_config_no_warnings().inspect(emit_warnings)
    }

    /// Same as [`LoadConfig::load_config`] but does not emit warnings.
    fn load_config_no_warnings(&self) -> Result<Config, ExtractConfigError> {
        self.load_config_unsanitized_no_warnings().map(Config::sanitized)
    }

    /// Load [`Config`] but do not sanitize. See [`Config::sanitized`] for more information.
    fn load_config_unsanitized(&self) -> Result<Config, ExtractConfigError> {
        self.load_config_unsanitized_no_warnings().inspect(emit_warnings)
    }

    /// Same as [`LoadConfig::load_config_unsanitized`] but also emits warnings generated
    fn load_config_unsanitized_no_warnings(&self) -> Result<Config, ExtractConfigError> {
        Config::from_provider(self.figment())
    }

    /// Load and sanitize the [`Config`], as well as extract [`EvmOpts`] from self
    fn load_config_and_evm_opts(&self) -> Result<(Config, EvmOpts)> {
        self.load_config_and_evm_opts_no_warnings().inspect(|(config, _)| emit_warnings(config))
    }

    /// Same as [`LoadConfig::load_config_and_evm_opts`] but also emits warnings generated
    fn load_config_and_evm_opts_no_warnings(&self) -> Result<(Config, EvmOpts)> {
        let figment = self.figment();

        let mut evm_opts = figment.extract::<EvmOpts>().map_err(ExtractConfigError::new)?;
        let config = Config::from_provider(figment)?.sanitized();

        // update the fork url if it was an alias
        if let Some(fork_url) = config.get_rpc_url() {
            trace!(target: "forge::config", ?fork_url, "Update EvmOpts fork url");
            evm_opts.fork_url = Some(fork_url?.into_owned());
        }

        Ok((config, evm_opts))
    }
}

impl<T> LoadConfig for T
where
    for<'a> Figment: From<&'a T>,
{
    fn figment(&self) -> Figment {
        self.into()
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
#[expect(clippy::too_many_arguments)]
pub async fn handle_traces(
    mut result: TraceResult,
    config: &Config,
    chain: Option<Chain>,
    contracts_bytecode: &HashMap<Address, Bytes>,
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

        if let Some(addr) = iter.next()
            && let (Ok(address), Some(label)) = (Address::from_str(addr), iter.next())
        {
            return Some((address, label.to_string()));
        }
        None
    });
    let config_labels = config.labels.clone().into_iter();

    let mut builder = CallTraceDecoderBuilder::new()
        .with_labels(labels.chain(config_labels))
        .with_signature_identifier(SignaturesIdentifier::from_config(config)?);
    let mut identifier = TraceIdentifiers::new().with_etherscan(config, chain)?;
    if let Some(contracts) = &known_contracts {
        builder = builder.with_known_contracts(contracts);
        identifier = identifier.with_local_and_bytecodes(contracts, contracts_bytecode);
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
            return Ok(());
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
        decode_trace_arena(arena, decoder).await;
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
pub fn cache_local_signatures(output: &ProjectCompileOutput) -> Result<()> {
    let Some(cache_dir) = Config::foundry_cache_dir() else {
        eyre::bail!("Failed to get `cache_dir` to generate local signatures.");
    };
    let path = cache_dir.join("signatures");
    let mut signatures = SignaturesCache::load(&path);
    for (_, artifact) in output.artifacts() {
        if let Some(abi) = &artifact.abi {
            signatures.extend_from_abi(abi);
        }

        // External libraries don't have functions included in the ABI, but `methodIdentifiers`.
        if let Some(method_identifiers) = &artifact.method_identifiers {
            signatures.extend(method_identifiers.iter().filter_map(|(signature, selector)| {
                Some((SelectorKind::Function(selector.parse().ok()?), signature.clone()))
            }));
        }
    }
    signatures.save(&path);
    Ok(())
}
