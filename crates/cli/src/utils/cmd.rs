use alloy_json_abi::JsonAbi as Abi;
use alloy_primitives::Address;
use eyre::{Result, WrapErr};
use foundry_common::{cli_warn, fs, TestFunctionExt};
use foundry_compilers::{
    artifacts::{CompactBytecode, CompactDeployedBytecode},
    cache::{CacheEntry, SolFilesCache},
    info::ContractInfo,
    utils::read_json_file,
    Artifact, ProjectCompileOutput,
};
use foundry_config::{error::ExtractConfigError, figment::Figment, Chain, Config, NamedChain};
use foundry_debugger::DebuggerArgs;
use foundry_evm::{
    debug::DebugArena,
    executors::{DeployResult, EvmError, ExecutionErr, RawCallResult},
    opts::EvmOpts,
    traces::{
        identifier::{EtherscanIdentifier, SignaturesIdentifier},
        CallTraceDecoder, CallTraceDecoderBuilder, TraceKind, Traces,
    },
};
use std::{fmt::Write, path::PathBuf, str::FromStr};
use yansi::Paint;

/// Given a `Project`'s output, removes the matching ABI, Bytecode and
/// Runtime Bytecode of the given contract.
#[track_caller]
pub fn remove_contract(
    output: &mut ProjectCompileOutput,
    info: &ContractInfo,
) -> Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    let contract = if let Some(contract) = output.remove_contract(info) {
        contract
    } else {
        let mut err = format!("could not find artifact: `{}`", info.name);
        if let Some(suggestion) =
            super::did_you_mean(&info.name, output.artifacts().map(|(name, _)| name)).pop()
        {
            if suggestion != info.name {
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
        .ok_or_else(|| eyre::eyre!("contract {} does not contain abi", info))?
        .into_owned();

    let bin = contract
        .get_bytecode()
        .ok_or_else(|| eyre::eyre!("contract {} does not contain bytecode", info))?
        .into_owned();

    let runtime = contract
        .get_deployed_bytecode()
        .ok_or_else(|| eyre::eyre!("contract {} does not contain deployed bytecode", info))?
        .into_owned();

    Ok((abi, bin, runtime))
}

/// Helper function for finding a contract by ContractName
// TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
// contract name?
pub fn get_cached_entry_by_name(
    cache: &SolFilesCache,
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
        return Ok(entry)
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
pub fn ensure_clean_constructor(abi: &Abi) -> Result<()> {
    if let Some(constructor) = &abi.constructor {
        if !constructor.inputs.is_empty() {
            eyre::bail!("Contract constructor should have no arguments. Add those arguments to  `run(...)` instead, and call it with `--sig run(...)`.");
        }
    }
    Ok(())
}

pub fn needs_setup(abi: &Abi) -> bool {
    let setup_fns: Vec<_> = abi.functions().filter(|func| func.name.is_setup()).collect();

    for setup_fn in setup_fns.iter() {
        if setup_fn.name != "setUp" {
            println!(
                "{} Found invalid setup function \"{}\" did you mean \"setUp()\"?",
                Paint::yellow("Warning:").bold(),
                setup_fn.signature()
            );
        }
    }

    setup_fns.len() == 1 && setup_fns[0].name == "setUp"
}

pub fn eta_key(state: &indicatif::ProgressState, f: &mut dyn Write) {
    write!(f, "{:.1}s", state.eta().as_secs_f64()).unwrap()
}

#[macro_export]
macro_rules! init_progress {
    ($local:expr, $label:expr) => {{
        let pb = indicatif::ProgressBar::new($local.len() as u64);
        let mut template =
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ".to_string();
        template += $label;
        template += " ({eta})";
        pb.set_style(
            indicatif::ProgressStyle::with_template(&template)
                .unwrap()
                .with_key("eta", $crate::utils::eta_key)
                .progress_chars("#>-"),
        );
        pb
    }};
}

#[macro_export]
macro_rules! update_progress {
    ($pb:ident, $index:expr) => {
        $pb.set_position(($index + 1) as u64);
    };
}

/// True if the network calculates gas costs differently.
pub fn has_different_gas_calc(chain_id: u64) -> bool {
    if let Some(chain) = Chain::from(chain_id).named() {
        return matches!(
            chain,
            NamedChain::Arbitrum |
                NamedChain::ArbitrumTestnet |
                NamedChain::ArbitrumGoerli |
                NamedChain::ArbitrumSepolia | 
                NamedChain::Moonbeam |
                NamedChain::Moonriver |
                NamedChain::Moonbase |
                NamedChain::MoonbeamDev
        )
    }
    false
}

/// True if it supports broadcasting in batches.
pub fn has_batch_support(chain_id: u64) -> bool {
    if let Some(chain) = Chain::from(chain_id).named() {
        return !matches!(
            chain,
            NamedChain::Arbitrum |
                NamedChain::ArbitrumTestnet |
                NamedChain::ArbitrumGoerli |
                NamedChain::ArbitrumSepolia
        )
    }
    true
}

/// Helpers for loading configuration.
///
/// This is usually implicitly implemented on a "&CmdArgs" struct via impl macros defined in
/// `forge_config` (See [`forge_config::impl_figment_convert`] for more details) and the impl
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
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        config
    }

    fn try_load_config_emit_warnings(self) -> Result<Config, ExtractConfigError> {
        let config = self.try_load_config()?;
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        Ok(config)
    }

    fn load_config_and_evm_opts_emit_warnings(self) -> Result<(Config, EvmOpts)> {
        let (config, evm_opts) = self.load_config_and_evm_opts()?;
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        Ok((config, evm_opts))
    }

    fn load_config_unsanitized_emit_warnings(self) -> Config {
        let config = self.load_config_unsanitized();
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        config
    }

    fn try_load_config_unsanitized_emit_warnings(self) -> Result<Config, ExtractConfigError> {
        let config = self.try_load_config_unsanitized()?;
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        Ok(config)
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
pub struct TraceResult {
    pub success: bool,
    pub traces: Traces,
    pub debug: DebugArena,
    pub gas_used: u64,
}

impl From<RawCallResult> for TraceResult {
    fn from(result: RawCallResult) -> Self {
        let RawCallResult { gas_used, traces, reverted, debug, .. } = result;

        Self {
            success: !reverted,
            traces: vec![(TraceKind::Execution, traces.expect("traces is None"))],
            debug: debug.unwrap_or_default(),
            gas_used,
        }
    }
}

impl From<DeployResult> for TraceResult {
    fn from(result: DeployResult) -> Self {
        let DeployResult { gas_used, traces, debug, .. } = result;

        Self {
            success: true,
            traces: vec![(TraceKind::Execution, traces.expect("traces is None"))],
            debug: debug.unwrap_or_default(),
            gas_used,
        }
    }
}

impl TryFrom<EvmError> for TraceResult {
    type Error = EvmError;

    fn try_from(err: EvmError) -> Result<Self, Self::Error> {
        match err {
            EvmError::Execution(err) => {
                let ExecutionErr { reverted, gas_used, traces, debug: run_debug, .. } = *err;
                Ok(TraceResult {
                    success: !reverted,
                    traces: vec![(TraceKind::Execution, traces.expect("traces is None"))],
                    debug: run_debug.unwrap_or_default(),
                    gas_used,
                })
            }
            _ => Err(err),
        }
    }
}

/// labels the traces, conditionally prints them or opens the debugger
pub async fn handle_traces(
    mut result: TraceResult,
    config: &Config,
    chain: Option<Chain>,
    labels: Vec<String>,
    verbose: bool,
    debug: bool,
) -> Result<()> {
    let mut etherscan_identifier = EtherscanIdentifier::new(config, chain)?;

    let labeled_addresses = labels.iter().filter_map(|label_str| {
        let mut iter = label_str.split(':');

        if let Some(addr) = iter.next() {
            if let (Ok(address), Some(label)) = (Address::from_str(addr), iter.next()) {
                return Some((address, label.to_string()))
            }
        }
        None
    });

    let mut decoder = CallTraceDecoderBuilder::new()
        .with_labels(labeled_addresses)
        .with_signature_identifier(SignaturesIdentifier::new(
            Config::foundry_cache_dir(),
            config.offline,
        )?)
        .build();

    for (_, trace) in &mut result.traces {
        decoder.identify(trace, &mut etherscan_identifier);
    }

    if debug {
        let sources = etherscan_identifier.get_compiled_contracts().await?;
        let debugger = DebuggerArgs {
            debug: vec![result.debug],
            decoder: &decoder,
            sources,
            breakpoints: Default::default(),
        };
        debugger.run()?;
    } else {
        print_traces(&mut result, &decoder, verbose).await?;
    }

    Ok(())
}

pub async fn print_traces(
    result: &mut TraceResult,
    decoder: &CallTraceDecoder,
    verbose: bool,
) -> Result<()> {
    if result.traces.is_empty() {
        panic!("No traces found")
    }

    println!("Traces:");
    for (_, trace) in &mut result.traces {
        decoder.decode(trace).await;
        if !verbose {
            println!("{trace}");
        } else {
            println!("{trace:#}");
        }
    }
    println!();

    if result.success {
        println!("{}", Paint::green("Transaction successfully executed."));
    } else {
        println!("{}", Paint::red("Transaction failed."));
    }

    println!("Gas used: {}", result.gas_used);
    Ok(())
}
