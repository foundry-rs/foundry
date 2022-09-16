use crate::suggestions;
use ethers::{
    abi::Abi,
    core::types::Chain,
    prelude::ArtifactId,
    solc::{
        artifacts::{CompactBytecode, CompactDeployedBytecode, ContractBytecodeSome},
        cache::{CacheEntry, SolFilesCache},
        info::ContractInfo,
        utils::read_json_file,
        Artifact, ProjectCompileOutput,
    },
};
use eyre::WrapErr;
use forge::executor::opts::EvmOpts;
use foundry_common::{cli_warn, fs, ContractsByArtifact, TestFunctionExt};
use foundry_config::{figment::Figment, Chain as ConfigChain, Config};
use std::{collections::BTreeMap, path::PathBuf};
use yansi::Paint;

/// Common trait for all cli commands
pub trait Cmd: clap::Parser + Sized {
    type Output;
    fn run(self) -> eyre::Result<Self::Output>;
}

/// Given a `Project`'s output, removes the matching ABI, Bytecode and
/// Runtime Bytecode of the given contract.
#[track_caller]
pub fn remove_contract(
    output: &mut ProjectCompileOutput,
    info: &ContractInfo,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    let contract = if let Some(contract) = output.remove_contract(info) {
        contract
    } else {
        let mut err = format!("could not find artifact: `{}`", info.name);
        if let Some(suggestion) =
            suggestions::did_you_mean(&info.name, output.artifacts().map(|(name, _)| name)).pop()
        {
            if suggestion != info.name {
                err = format!(
                    r#"{}

        Did you mean `{}`?"#,
                    err, suggestion
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
) -> eyre::Result<(PathBuf, CacheEntry)> {
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

    let mut err = format!("could not find artifact: `{}`", name);
    if let Some(suggestion) = suggestions::did_you_mean(name, &alternatives).pop() {
        err = format!(
            r#"{}

        Did you mean `{}`?"#,
            err, suggestion
        );
    }
    eyre::bail!(err)
}

pub fn u32_validator(min: u32, max: u32) -> impl FnMut(&str) -> eyre::Result<()> {
    move |v: &str| -> eyre::Result<()> {
        let v = v.parse::<u32>()?;
        if v >= min && v <= max {
            Ok(())
        } else {
            Err(eyre::eyre!("Expected between {} and {} inclusive.", min, max))
        }
    }
}

/// Returns error if constructor has arguments.
pub fn ensure_clean_constructor(abi: &Abi) -> eyre::Result<()> {
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

pub fn unwrap_contracts(
    contracts: &BTreeMap<ArtifactId, ContractBytecodeSome>,
    deployed_code: bool,
) -> ContractsByArtifact {
    ContractsByArtifact(
        contracts
            .iter()
            .filter_map(|(id, c)| {
                let bytecode = if deployed_code {
                    c.deployed_bytecode.clone().into_bytes()
                } else {
                    c.bytecode.clone().object.into_bytes()
                };

                if let Some(bytecode) = bytecode {
                    return Some((id.clone(), (c.abi.clone(), bytecode.to_vec())))
                }
                None
            })
            .collect(),
    )
}

#[macro_export]
macro_rules! init_progress {
    ($local:expr, $label:expr) => {{
        let pb = ProgressBar::new($local.len() as u64);
        let mut template =
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ".to_string();
        template += $label;
        template += " ({eta})";
        pb.set_style(
            ProgressStyle::with_template(&template)
                .unwrap()
                .with_key("eta", |state| format!("{:.1}s", state.eta().as_secs_f64()))
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
pub fn has_different_gas_calc(chain: u64) -> bool {
    if let ConfigChain::Named(chain) = ConfigChain::from(chain) {
        return matches!(chain, Chain::Arbitrum | Chain::ArbitrumTestnet)
    }
    false
}

/// True if it supports broadcasting in batches.
pub fn has_batch_support(chain: u64) -> bool {
    if let ConfigChain::Named(chain) = ConfigChain::from(chain) {
        return !matches!(
            chain,
            Chain::Arbitrum | Chain::ArbitrumTestnet | Chain::Optimism | Chain::OptimismKovan
        )
    }
    true
}

/// Helpers for loading configuration.
///
/// This is usually implicity implemented on a "&CmdArgs" struct via impl macros defined in
/// `forge_config` (See [`forge_config::impl_figment_convert`] for more details) and the impl
/// definition on `T: Into<Config> + Into<Figment>` below.
///
/// Each function also has an `emit_warnings` form which does the same thing as its counterpart but
/// also prints `Config::__warnings` to stderr
pub trait LoadConfig {
    /// Load and sanitize the [`Config`] based on the options provided in self
    fn load_config(self) -> Config;
    /// Load and sanitize the [`Config`], as well as extract [`EvmOpts`] from self
    fn load_config_and_evm_opts(self) -> eyre::Result<(Config, EvmOpts)>;
    /// Load [`Config`] but do not sanitize. See [`Config::sanitized`] for more information
    fn load_config_unsanitized(self) -> Config;

    /// Same as [`LoadConfig::load_config`] but also emits warnings generated
    fn load_config_emit_warnings(self) -> Config;
    /// Same as [`LoadConfig::load_config_and_evm_opts`] but also emits warnings generated
    fn load_config_and_evm_opts_emit_warnings(self) -> eyre::Result<(Config, EvmOpts)>;
    /// Same as [`LoadConfig::load_config_unsanitized`] but also emits warnings generated
    fn load_config_unsanitized_emit_warnings(self) -> Config;
}

impl<T> LoadConfig for T
where
    T: Into<Config> + Into<Figment>,
{
    fn load_config(self) -> Config {
        self.into()
    }
    fn load_config_and_evm_opts(self) -> eyre::Result<(Config, EvmOpts)> {
        let figment: Figment = self.into();
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();

        // update the fork url if it was an alias
        if let Some(fork_url) = config.get_rpc_url() {
            evm_opts.fork_url = Some(fork_url?.into_owned());
        }

        Ok((config, evm_opts))
    }
    fn load_config_unsanitized(self) -> Config {
        let figment: Figment = self.into();
        Config::from_provider(figment)
    }
    fn load_config_emit_warnings(self) -> Config {
        let config = self.load_config();
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        config
    }
    fn load_config_and_evm_opts_emit_warnings(self) -> eyre::Result<(Config, EvmOpts)> {
        let (config, evm_opts) = self.load_config_and_evm_opts()?;
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        Ok((config, evm_opts))
    }
    fn load_config_unsanitized_emit_warnings(self) -> Config {
        let config = self.load_config_unsanitized();
        config.__warnings.iter().for_each(|w| cli_warn!("{w}"));
        config
    }
}

/// Read contract constructor arguments from the given file.
pub fn read_constructor_args_file(constructor_args_path: PathBuf) -> eyre::Result<Vec<String>> {
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
