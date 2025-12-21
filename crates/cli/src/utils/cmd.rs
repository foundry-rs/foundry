use alloy_json_abi::JsonAbi;
use eyre::{Result, WrapErr};
use foundry_common::{TestFunctionExt, fs, fs::json_files, selectors::SelectorKind, shell};
use foundry_compilers::{
    Artifact, ArtifactId, ProjectCompileOutput,
    artifacts::{CompactBytecode, Settings},
    cache::{CacheEntry, CompilerCache},
    utils::read_json_file,
};
use foundry_config::{Chain, Config, NamedChain, error::ExtractConfigError, figment::Figment};
use foundry_evm::{
    executors::{DeployResult, EvmError, RawCallResult},
    opts::EvmOpts,
    traces::{
        CallTraceDecoder, TraceKind, Traces, decode_trace_arena, identifier::SignaturesCache,
        prune_trace_depth, render_trace_arena_inner,
    },
};
use std::{
    fmt::Write,
    path::{Path, PathBuf},
};
use yansi::Paint;

/// Given a `Project`'s output, finds the contract by path and name and returns its
/// ABI, creation bytecode, and `ArtifactId`.
#[track_caller]
pub fn find_contract_artifacts(
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
                    | NamedChain::Monad
                    | NamedChain::MonadTestnet
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

pub async fn print_traces(
    result: &mut TraceResult,
    decoder: &CallTraceDecoder,
    verbose: bool,
    state_changes: bool,
    trace_depth: Option<usize>,
) -> Result<()> {
    let traces = result.traces.as_mut().expect("No traces found");

    if !shell::is_json() {
        sh_println!("Traces:")?;
    }

    for (_, arena) in traces {
        decode_trace_arena(arena, decoder).await;

        if let Some(trace_depth) = trace_depth {
            prune_trace_depth(arena, trace_depth);
        }

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

/// Traverses all files at `folder_path`, parses any JSON ABI files found,
/// and caches their function/event/error signatures to the local signatures cache.
pub fn cache_signatures_from_abis(folder_path: impl AsRef<Path>) -> Result<()> {
    let Some(cache_dir) = Config::foundry_cache_dir() else {
        eyre::bail!("Failed to get `cache_dir` to generate local signatures.");
    };
    let path = cache_dir.join("signatures");
    let mut signatures = SignaturesCache::load(&path);

    json_files(folder_path.as_ref())
        .filter_map(|path| std::fs::read_to_string(&path).ok())
        .filter_map(|content| serde_json::from_str::<JsonAbi>(&content).ok())
        .for_each(|json_abi| signatures.extend_from_abi(&json_abi));

    signatures.save(&path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_cache_signatures_from_abis() {
        let temp_dir = tempdir().unwrap();
        let abi_json = r#"[
              {
                  "type": "function",
                  "name": "myCustomFunction",
                  "inputs": [{"name": "amount", "type": "uint256"}],
                  "outputs": [],
                  "stateMutability": "nonpayable"
              },
              {
                  "type": "event",
                  "name": "MyCustomEvent",
                  "inputs": [{"name": "value", "type": "uint256", "indexed": false}],
                  "anonymous": false
              },
              {
                  "type": "error",
                  "name": "MyCustomError",
                  "inputs": [{"name": "code", "type": "uint256"}]
              }
          ]"#;

        let abi_path = temp_dir.path().join("test.json");
        fs::write(&abi_path, abi_json).unwrap();

        cache_signatures_from_abis(temp_dir.path()).unwrap();

        let cache_dir = Config::foundry_cache_dir().unwrap();
        let cache_path = cache_dir.join("signatures");
        let cache = SignaturesCache::load(&cache_path);

        let func_selector: alloy_primitives::Selector = "0x2e2dbaf7".parse().unwrap();
        assert!(cache.contains_key(&SelectorKind::Function(func_selector)));

        let event_selector: alloy_primitives::B256 =
            "0x8cc20c47f3a2463817352f75dec0dbf43a7a771b5f6817a92bd5724c1f4aa745".parse().unwrap();
        assert!(cache.contains_key(&SelectorKind::Event(event_selector)));

        let error_selector: alloy_primitives::Selector = "0xd35f45de".parse().unwrap();
        assert!(cache.contains_key(&SelectorKind::Error(error_selector)));
    }
}
