//! # foundry-script
//!
//! Smart contract scripting.

#![recursion_limit = "256"]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

use crate::{broadcast::BundledState, runner::ScriptRunner};
use alloy_json_abi::{Function, JsonAbi};
use alloy_network::Network;
use alloy_primitives::{
    Address, Bytes, Log, U256, hex,
    map::{AddressHashMap, HashMap},
};
use alloy_signer::Signer;
use broadcast::next_nonce_resolved;
use build::PreprocessedState;
use clap::{Parser, ValueHint, builder::RangedU64ValueParser};
use dialoguer::Confirm;
use eyre::{ContextCompat, Result};
use forge_script_sequence::{AdditionalContract, NestedValue};
use forge_verify::{RetryArgs, VerifierArgs};
use foundry_cli::{
    opts::{BuildOpts, EvmArgs, GlobalArgs, TempoOpts, TracingArgs},
    utils::LoadConfig,
};
use foundry_common::{
    ContractsByArtifact, SELECTOR_LEN,
    abi::{encode_function_args, get_func},
    compile::ContractSizeLimits,
    shell,
};
use foundry_compilers::{ArtifactId, artifacts::output_selection::ContractOutputSelection};
use foundry_config::{
    Config, Eip1559FeeEstimatePreset, FoundryHardfork, figment,
    figment::{
        Metadata, Profile, Provider,
        value::{Dict, Map},
    },
};
use foundry_debugger::DebuggerLayout;
#[cfg(feature = "monad")]
use foundry_evm::core::evm::MonadEvmNetwork;
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    backend::Backend,
    core::{
        Breakpoints, FoundryTransaction,
        evm::{EthEvmNetwork, EvmEnvFor, FoundryEvmNetwork, SpecFor, TempoEvmNetwork, TxEnvFor},
        fork::ResolvedFork,
    },
    executors::ExecutorBuilder,
    inspectors::{
        CheatsConfig,
        cheatcodes::{BroadcastableTransactions, Wallets},
    },
    opts::{EvmOpts, ExecutionSpecContext, resolve_execution_spec},
    revm::interpreter::InstructionResult,
    traces::{InternalTraceMode, TraceRequirements, Traces},
};
use foundry_evm_networks::NetworkConfigs;
use foundry_wallets::MultiWalletOpts;
use serde::Serialize;
use std::path::PathBuf;

mod broadcast;
mod build;
mod execute;
mod library_deployments;
mod multi_sequence;
mod progress;
mod providers;
mod receipts;
mod runner;
mod sequence;
mod session;
mod simulate;
mod transaction;
mod verify;
mod wallet_session;

pub use wallet_session::ScriptWalletSessionArgs;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(ScriptArgs, build, evm);

const DEFAULT_CONFIRMATIONS: u64 = 1;

/// CLI arguments for `forge script`.
#[derive(Clone, Debug, Default, Parser)]
pub struct ScriptArgs {
    // Include global options for users of this struct.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// The contract you want to run. Either the file path or contract name.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[arg(value_hint = ValueHint::FilePath)]
    pub path: String,

    /// Arguments to pass to the script function.
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[arg(long, visible_alias = "tc", value_name = "CONTRACT_NAME")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[arg(long, short, default_value = "run")]
    pub sig: String,

    /// Max priority fee per gas for EIP1559 transactions.
    #[arg(
        long,
        env = "ETH_PRIORITY_GAS_PRICE",
        value_parser = foundry_cli::utils::parse_ether_value,
        value_name = "PRICE"
    )]
    pub priority_gas_price: Option<U256>,

    /// Use legacy transactions instead of EIP1559 ones.
    ///
    /// This is auto-enabled for common networks without EIP1559.
    #[arg(long)]
    pub legacy: bool,

    /// How to estimate EIP-1559 fees: `low`, `market` (default), or `aggressive`.
    ///
    /// The preset sets the priority-fee percentile and the `maxFeePerGas` buffer
    /// (`low`: `base_fee * 1.5`, others: `* 2`); `low`'s tighter buffer is more
    /// likely to stall if the base fee rises. `--with-gas-price` and
    /// `--priority-gas-price` override only `maxFeePerGas` and
    /// `maxPriorityFeePerGas` respectively. Ignored for `--legacy`.
    #[arg(long = "estimate", value_name = "PRESET")]
    pub eip1559_fee_estimate: Option<Eip1559FeeEstimatePreset>,

    /// Broadcasts the transactions.
    #[arg(long)]
    pub broadcast: bool,

    /// Batch all broadcast transactions into a single Tempo batch transaction.
    ///
    /// When enabled, all vm.broadcast() calls are collected and sent as a single
    /// atomic type 0x76 transaction instead of individual transactions.
    /// This provides atomicity (all-or-nothing execution) and gas savings.
    #[arg(long)]
    pub batch: bool,

    /// Tempo transaction options.
    #[command(flatten)]
    pub tempo: TempoOpts,

    /// Create a temporary Tempo wallet session, run this script with it, then revoke it.
    #[command(flatten)]
    pub wallet_session: ScriptWalletSessionArgs,

    /// Skips on-chain simulation.
    #[arg(long)]
    pub skip_simulation: bool,

    /// Relative percentage to multiply gas estimates by.
    #[arg(long, short, default_value = "130")]
    pub gas_estimate_multiplier: u64,

    /// Override the sender's initial nonce for script execution and transaction generation.
    #[arg(
        long,
        value_name = "NONCE",
        value_parser = RangedU64ValueParser::<u64>::new().range(..u64::MAX),
    )]
    pub sender_nonce: Option<u64>,

    /// Send via `eth_sendTransaction` using the `--sender` argument as sender.
    #[arg(
        long,
        conflicts_with_all = &["private_key", "private_keys", "ledger", "trezor", "aws", "browser"],
    )]
    pub unlocked: bool,

    /// Resumes submitting transactions that failed or timed-out previously.
    ///
    /// It DOES NOT simulate the script again and it expects nonces to have remained the same.
    ///
    /// Example: If transaction N has a nonce of 22, then the account should have a nonce of 22,
    /// otherwise it fails.
    #[arg(long)]
    pub resume: bool,

    /// If present, --resume or --verify will be assumed to be a multi chain deployment.
    #[arg(long)]
    pub multi: bool,

    /// Open the script in the debugger.
    ///
    /// Takes precedence over broadcast.
    #[arg(long)]
    pub debug: bool,

    /// Debugger layout to use.
    #[arg(long = "debug-layout", requires = "debug", value_enum)]
    pub debug_layout: Option<DebuggerLayout>,

    /// Dumps all debugger steps to file.
    #[arg(
        long,
        requires = "debug",
        value_hint = ValueHint::FilePath,
        value_name = "PATH"
    )]
    pub dump: Option<PathBuf>,

    /// Makes sure a transaction is sent,
    /// only after its previous one has been confirmed and succeeded.
    #[arg(long)]
    pub slow: bool,

    /// The number of confirmations to wait for after broadcasting.
    #[arg(
        long,
        default_value_t = DEFAULT_CONFIRMATIONS,
        value_parser = RangedU64ValueParser::<u64>::new().range(1..),
    )]
    pub confirmations: u64,

    /// Disables interactive prompts that might appear when deploying big contracts.
    ///
    /// For more info on the contract size limit, see EIP-170: <https://eips.ethereum.org/EIPS/eip-170>
    #[arg(long)]
    pub non_interactive: bool,

    /// Disables the contract size limit during script execution.
    #[arg(long)]
    pub disable_code_size_limit: bool,

    #[command(flatten)]
    pub tracing: TracingArgs,

    /// The Etherscan (or equivalent) API key
    #[arg(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    pub etherscan_api_key: Option<String>,

    /// Verifies all the contracts found in the receipts of a script, if any.
    #[arg(long, requires = "broadcast")]
    pub verify: bool,

    /// Opt-in verification of contracts deployed by external factories using explorer source.
    #[arg(long, requires = "verify", conflicts_with_all = ["skip_simulation", "offline"])]
    pub verify_external: bool,

    /// Gas price for legacy transactions, or max fee per gas for EIP1559 transactions, either
    /// specified in wei, or as a string with a unit type.
    ///
    /// Examples: 1ether, 10gwei, 0.01ether
    #[arg(
        long,
        env = "ETH_GAS_PRICE",
        value_parser = foundry_cli::utils::parse_ether_value,
        value_name = "PRICE",
    )]
    pub with_gas_price: Option<U256>,

    /// Timeout to use for broadcasting transactions.
    #[arg(long, env = "ETH_TIMEOUT")]
    pub timeout: Option<u64>,

    #[command(flatten)]
    pub build: BuildOpts,

    #[command(flatten)]
    pub wallets: MultiWalletOpts,

    #[command(flatten)]
    pub evm: EvmArgs,

    #[command(flatten)]
    pub verifier: VerifierArgs,

    #[command(flatten)]
    pub retry: RetryArgs,
}

impl ScriptArgs {
    fn has_tempo_session(&self) -> Result<bool> {
        Ok(self.tempo.session_id()?.is_some())
    }

    /// Loads config, resolves evm_opts (including network inference from fork), and returns them.
    async fn resolved_evm_opts(&self) -> Result<(Config, EvmOpts)> {
        let (mut config, mut evm_opts) = self.load_config_and_evm_opts()?;

        if self.tempo.is_tempo() || self.has_tempo_session()? {
            // If Tempo tx options or a session are set, select the Tempo network.
            evm_opts.networks = NetworkConfigs::with_tempo();
        }
        // Discover endpoint source identity and exact hardfork even when execution network is
        // explicit. Without an explicit selection this also infers the execution network.
        evm_opts.infer_network_from_fork().await?;
        config.networks = evm_opts.networks;

        Ok((config, evm_opts))
    }

    async fn preprocess<FEN: FoundryEvmNetwork>(
        self,
        mut config: Config,
        mut evm_opts: EvmOpts,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<PreprocessedState<FEN>> {
        let args = self;
        let mut tempo = args.tempo.clone();

        let session_sender = if args.resume {
            None
        } else {
            // Initial scripts may only reveal multi-chain transactions during execution. Use the
            // session root as the script sender here and validate chain scope during broadcast.
            tempo.session_sender_for_multi_wallet(&args.wallets, args.evm.sender)?
        };

        let script_wallets = Wallets::new(args.wallets.get_multi_wallet().await?, args.evm.sender);
        let browser_wallet = args.wallets.browser_signer::<FEN::Network>().await?;

        if let Some(sender) = session_sender {
            evm_opts.sender = sender;
        } else if let Some(sender) = args.maybe_load_private_key()? {
            evm_opts.sender = sender;
        } else if args.evm.sender.is_none() {
            // If no sender was explicitly set via --sender, auto-detect it from available signers:
            // use the sole signer's address if there's exactly one, or fall back to the browser
            // wallet address if present.
            let addresses = script_wallets.addresses();
            if addresses.len() == 1 {
                evm_opts.sender = addresses[0];
            } else if let Some(signer) = browser_wallet.as_ref().map(|b| b.address()) {
                evm_opts.sender = signer
            }
        }

        tempo.resolve_expires();
        config.tracing = args.tracing.resolve(&config.tracing, evm_opts.verbosity);
        if args.debug && !config.extra_output.contains(&ContractOutputSelection::StorageLayout) {
            config.extra_output.push(ContractOutputSelection::StorageLayout);
        }

        let script_config = ScriptConfig::new(
            config,
            evm_opts,
            executor_builder,
            args.batch,
            tempo,
            args.sender_nonce,
        )
        .await?;
        Ok(PreprocessedState { args, script_config, script_wallets, browser_wallet })
    }

    /// Executes the script
    #[allow(clippy::large_stack_frames)]
    pub async fn run_script(self) -> Result<()> {
        trace!(target: "script", "executing script command");

        if self.tempo.sponsor_url.is_some() {
            eyre::bail!(
                "--sponsor-url is not supported by forge script; use --tempo.sponsor with \
                 --tempo.sponsor-signer or --tempo.sponsor-sig"
            );
        }

        if self.wallet_session.enabled {
            return self.run_wallet_session_wrapper();
        }

        let (config, evm_opts) = self.resolved_evm_opts().await?;

        let is_tempo = evm_opts.networks.is_tempo();

        if self.batch && !is_tempo {
            eyre::bail!("--batch mode is only supported on Tempo networks");
        }

        if self.unlocked && self.has_tempo_session()? {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --unlocked");
        }

        // Box each branch's future to keep its large async state off `run_script`'s future;
        // otherwise `run_command` trips `clippy::large_stack_frames` by a small margin.
        if is_tempo {
            let batch = self.batch;
            return Box::pin(async move {
                let bundled = match self
                    .prepare_bundled::<TempoEvmNetwork>(
                        config,
                        evm_opts,
                        ExecutorBuilder::<TempoEvmNetwork>::new(),
                    )
                    .await?
                {
                    Some(bundled) => bundled,
                    None => return Ok(()),
                };
                // batch mode owns its own pending recovery inside broadcast_batch(); running the
                // generic wait_for_pending() first would race with that and could double-process
                // an already-confirmed batch hash.
                let bundled = if batch { bundled } else { bundled.wait_for_pending().await? };
                let broadcasted = if batch {
                    bundled.broadcast_batch().await?
                } else {
                    bundled.broadcast().await?
                };
                if broadcasted.args.verify {
                    broadcasted.verify().await?;
                }
                Ok(())
            })
            .await;
        }

        #[cfg(feature = "base")]
        if evm_opts.networks.is_base() {
            return Box::pin(self.run_generic_script::<foundry_evm::core::evm::BaseEvmNetwork>(
                config,
                evm_opts,
                ExecutorBuilder::<foundry_evm::core::evm::BaseEvmNetwork>::new(),
            ))
            .await;
        }

        #[cfg(feature = "monad")]
        if evm_opts.networks.is_monad() {
            return Box::pin(self.run_generic_script::<MonadEvmNetwork>(
                config,
                evm_opts,
                ExecutorBuilder::<MonadEvmNetwork>::new(),
            ))
            .await;
        }

        #[cfg(feature = "optimism")]
        if evm_opts.networks.is_optimism() {
            return Box::pin(self.run_generic_script::<OpEvmNetwork>(
                config,
                evm_opts,
                ExecutorBuilder::<OpEvmNetwork>::new(),
            ))
            .await;
        }

        Box::pin(self.run_generic_script::<EthEvmNetwork>(
            config,
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
        ))
        .await
    }

    /// Prepares the bundled state (compile, simulate, bundle) and returns it
    /// for broadcasting, or returns `None` if there's nothing to broadcast
    /// (e.g., debug mode, no transactions, missing RPCs).
    #[allow(clippy::large_stack_frames)]
    async fn prepare_bundled<FEN: FoundryEvmNetwork>(
        self,
        config: Config,
        evm_opts: EvmOpts,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<Option<BundledState<FEN>>> {
        let state = self.preprocess::<FEN>(config, evm_opts, executor_builder).await?;
        let create2_deployer = state.script_config.evm_opts.create2_deployer;
        let compiled = state.compile()?;

        // Move from `CompiledState` to `BundledState` either by resuming or executing and
        // simulating script.
        let bundled = if compiled.args.resume {
            compiled.resume().await?
        } else {
            // Drive state machine to point at which we have everything needed for simulation.
            let pre_simulation = compiled
                .link()
                .await?
                .prepare_execution()
                .await?
                .execute()
                .await?
                .prepare_simulation()
                .await?
                .optimize_library_deployments()
                .await?;

            if pre_simulation.args.debug {
                return match pre_simulation.args.dump.clone() {
                    Some(path) => pre_simulation.dump_debugger(&path).map(|_| None),
                    None => pre_simulation.run_debugger().map(|_| None),
                };
            }

            if shell::is_json() {
                pre_simulation.show_json().await?;
            } else {
                pre_simulation.show_traces().await?;
            }

            // Ensure that we have transactions to simulate/broadcast, otherwise exit early to avoid
            // hard error.
            if pre_simulation
                .execution_result
                .transactions
                .as_ref()
                .is_none_or(|txs| txs.is_empty())
            {
                if pre_simulation.args.broadcast {
                    sh_warn!("No transactions to broadcast.")?;
                }

                return Ok(None);
            }

            // Check if there are any missing RPCs and exit early to avoid hard error.
            if pre_simulation.execution_artifacts.rpc_data.missing_rpc {
                if !shell::is_json() {
                    sh_println!("\nIf you wish to simulate on-chain transactions pass a RPC URL.")?;
                }

                return Ok(None);
            }

            let size_limits = pre_simulation.args.contract_size_limits::<FEN>(
                &pre_simulation.script_config.config,
                &pre_simulation.script_config.evm_opts,
            );
            pre_simulation.args.check_contract_sizes(
                size_limits,
                &pre_simulation.execution_result,
                &pre_simulation.build_data.known_contracts,
                create2_deployer,
            )?;

            pre_simulation.fill_metadata().await?.bundle().await?
        };

        // Exit early in case user didn't provide any broadcast/verify related flags.
        if !bundled.args.should_broadcast() {
            if !shell::is_json() {
                if shell::verbosity() >= 4 {
                    sh_println!("\n=== Transactions that will be broadcast ===\n")?;
                    bundled.sequence.show_transactions()?;
                }

                sh_println!(
                    "\nSIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more."
                )?;
            }
            return Ok(None);
        }

        // Exit early if something is wrong with verification options.
        if bundled.args.verify {
            bundled.verify_preflight_check().await?;
        }

        Ok(Some(bundled))
    }

    async fn run_generic_script<FEN: FoundryEvmNetwork>(
        self,
        config: Config,
        evm_opts: EvmOpts,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<()> {
        let bundled = match self.prepare_bundled::<FEN>(config, evm_opts, executor_builder).await? {
            Some(bundled) => bundled,
            None => return Ok(()),
        };

        // Wait for pending txes and broadcast others.
        let broadcasted = bundled.wait_for_pending().await?.broadcast().await?;

        if broadcasted.args.verify {
            broadcasted.verify().await?;
        }

        Ok(())
    }

    /// In case the user has loaded *only* one private-key or a single remote signer (e.g.,
    /// Turnkey), we can assume that they're using it as the `--sender`.
    fn maybe_load_private_key(&self) -> Result<Option<Address>> {
        if let Some(turnkey_address) = self.wallets.turnkey_address() {
            return Ok(Some(turnkey_address));
        }

        let maybe_sender = self
            .wallets
            .private_keys()?
            .filter(|pks| pks.len() == 1)
            .map(|pks| pks.first().unwrap().address());
        Ok(maybe_sender)
    }

    /// Returns the Function and calldata based on the signature
    ///
    /// If the `sig` is a valid human-readable function we find the corresponding function in the
    /// `abi` If the `sig` is valid hex, we assume it's calldata and try to find the
    /// corresponding function by matching the selector, first 4 bytes in the calldata.
    ///
    /// Note: We assume that the `sig` is already stripped of its prefix, See [`ScriptArgs`]
    fn get_method_and_calldata(&self, abi: &JsonAbi) -> Result<(Function, Bytes)> {
        if let Ok(decoded) = hex::decode(&self.sig) {
            let selector = &decoded[..SELECTOR_LEN];
            let func =
                abi.functions().find(|func| selector == &func.selector()[..]).ok_or_else(|| {
                    eyre::eyre!(
                        "Function selector `{}` not found in the ABI",
                        hex::encode(selector)
                    )
                })?;
            return Ok((func.clone(), decoded.into()));
        }

        let func = if self.sig.contains('(') {
            let func = get_func(&self.sig)?;
            abi.functions()
                .find(|&abi_func| abi_func.selector() == func.selector())
                .wrap_err(format!("Function `{}` is not implemented in your script.", self.sig))?
        } else {
            let matching_functions =
                abi.functions().filter(|func| func.name == self.sig).collect::<Vec<_>>();
            match matching_functions.len() {
                0 => {
                    eyre::bail!("Function `{}` not found in the ABI", self.sig);
                }
                1 => matching_functions[0],
                2.. => {
                    eyre::bail!(
                        "Multiple functions with the same name `{}` found in the ABI",
                        self.sig
                    );
                }
            }
        };
        let data = encode_function_args(func, &self.args)?;

        Ok((func.clone(), data.into()))
    }

    /// Checks if the transaction is a deployment with a size above the active EVM specification's
    /// contract size limit or the specified `code_size_limit`.
    ///
    /// If `self.broadcast` is enabled, it asks confirmation of the user. Otherwise, it just warns
    /// the user.
    fn check_contract_sizes<N: Network>(
        &self,
        size_limits: ContractSizeLimits,
        result: &ScriptResult<N>,
        known_contracts: &ContractsByArtifact,
        create2_deployer: Address,
    ) -> Result<()> {
        // If disable-code-size-limit flag is enabled then skip the size check
        if self.disable_code_size_limit {
            return Ok(());
        }

        // (name, &init, &deployed)[]
        let mut bytecodes: Vec<(String, &[u8], &[u8])> = vec![];

        // From artifacts
        for (artifact, contract) in known_contracts.iter() {
            let Some(bytecode) = contract.bytecode() else { continue };
            let Some(deployed_bytecode) = contract.deployed_bytecode() else { continue };
            bytecodes.push((artifact.name.clone(), bytecode, deployed_bytecode));
        }

        // From traces
        let create_nodes = result.traces.iter().flat_map(|(_, traces)| {
            traces.nodes().iter().filter(|node| node.trace.kind.is_any_create())
        });
        let mut unknown_c = 0usize;
        for node in create_nodes {
            let init_code = &node.trace.data;
            let deployed_code = &node.trace.output;
            if !bytecodes.iter().any(|(_, b, _)| *b == init_code.as_ref()) {
                bytecodes.push((format!("Unknown{unknown_c}"), init_code, deployed_code));
                unknown_c += 1;
            }
        }

        let mut prompt_user = false;
        let max_size = size_limits.runtime;

        for (data, to) in result.transactions.iter().flat_map(|txes| {
            txes.iter().filter_map(|tx| {
                tx.transaction
                    .input()
                    .filter(|data| data.len() > max_size)
                    .map(|data| (data, tx.transaction.to()))
            })
        }) {
            let mut offset = 0;

            // Find if it's a CREATE or CREATE2. Otherwise, skip transaction.
            if let Some(to) = to {
                if to == create2_deployer {
                    // Size of the salt prefix.
                    offset = 32;
                } else {
                    continue;
                }
            }

            // Find artifact with a deployment code same as the data.
            if let Some((name, _, deployed_code)) =
                bytecodes.iter().find(|(_, init_code, _)| *init_code == &data[offset..])
            {
                let deployment_size = deployed_code.len();

                if deployment_size > max_size {
                    prompt_user = self.should_broadcast();
                    sh_err!(
                        "`{name}` is above the contract size limit ({deployment_size} > {max_size})."
                    )?;
                }
            }
        }

        // Only prompt if we're broadcasting and we've not disabled interactivity.
        if prompt_user
            && !self.non_interactive
            && !Confirm::new().with_prompt("Do you wish to continue?".to_string()).interact()?
        {
            eyre::bail!("User canceled the script.");
        }

        Ok(())
    }

    fn contract_size_limits<FEN: FoundryEvmNetwork>(
        &self,
        config: &Config,
        evm_opts: &EvmOpts,
    ) -> ContractSizeLimits {
        self.evm
            .env
            .code_size_limit
            .or(evm_opts.env.code_size_limit)
            .or(config.code_size_limit)
            .map(ContractSizeLimits::with_runtime_limit)
            .or_else(|| {
                evm_opts
                    .networks
                    .contract_size_limits()
                    .map(|limits| ContractSizeLimits::new(limits.runtime, limits.initcode))
            })
            .unwrap_or_else(|| {
                let spec_id: SpecFor<FEN> = config.evm_spec_id();
                ContractSizeLimits::for_spec_id(spec_id.into())
            })
    }

    /// We only broadcast transactions if --broadcast, --resume, or --verify was passed.
    const fn should_broadcast(&self) -> bool {
        self.broadcast || self.resume || self.verify
    }
}

impl Provider for ScriptArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Script Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();

        if let Some(etherscan_api_key) =
            self.etherscan_api_key.as_ref().filter(|s| !s.trim().is_empty())
        {
            dict.insert(
                "etherscan_api_key".to_string(),
                figment::value::Value::from(etherscan_api_key.clone()),
            );
        }

        if let Some(timeout) = self.timeout {
            dict.insert("transaction_timeout".to_string(), timeout.into());
        }

        if let Some(preset) = self.eip1559_fee_estimate {
            dict.insert(
                "eip1559_fee_estimate".to_string(),
                figment::value::Value::from(preset.to_string()),
            );
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

#[derive(Serialize, Clone)]
#[serde(bound = "")]
pub struct ScriptResult<N: Network> {
    pub success: bool,
    #[serde(rename = "raw_logs")]
    pub logs: Vec<Log>,
    pub traces: Traces,
    pub gas_used: u64,
    pub labeled_addresses: AddressHashMap<String>,
    #[serde(skip)]
    pub debug_bytecodes: AddressHashMap<Bytes>,
    #[serde(skip)]
    pub transactions: Option<BroadcastableTransactions<N>>,
    pub returned: Bytes,
    #[serde(skip)]
    pub exit_reason: Option<InstructionResult>,
    pub address: Option<Address>,
    #[serde(skip)]
    pub breakpoints: Breakpoints,
}

impl<N: Network> Default for ScriptResult<N> {
    fn default() -> Self {
        Self {
            success: Default::default(),
            logs: Default::default(),
            traces: Default::default(),
            gas_used: Default::default(),
            labeled_addresses: Default::default(),
            debug_bytecodes: Default::default(),
            transactions: Default::default(),
            returned: Default::default(),
            exit_reason: Default::default(),
            address: Default::default(),
            breakpoints: Default::default(),
        }
    }
}

impl<N: Network> ScriptResult<N> {
    pub fn get_created_contracts(
        &self,
        known_contracts: &ContractsByArtifact,
    ) -> Vec<AdditionalContract> {
        self.traces
            .iter()
            .flat_map(|(_, traces)| {
                let nodes = traces.nodes();
                nodes.iter().filter_map(|node| {
                    if !node.trace.kind.is_any_create() || !node.trace.success {
                        return None;
                    }

                    let mut creator_code_addresses = Vec::new();
                    let mut parent = node.parent;
                    while let Some(parent_idx) = parent {
                        let ancestor = &nodes[parent_idx];
                        if !ancestor.trace.success {
                            return None;
                        }
                        if !ancestor.trace.kind.is_any_create()
                            && !creator_code_addresses.contains(&ancestor.trace.address)
                        {
                            creator_code_addresses.push(ancestor.trace.address);
                        }
                        parent = ancestor.parent;
                    }

                    let init_code = node.trace.data.clone();
                    let contract_name = known_contracts
                        .find_by_creation_code(init_code.as_ref())
                        .map(|artifact| artifact.0.name.clone());
                    Some(AdditionalContract {
                        call_kind: node.trace.kind,
                        address: node.trace.address,
                        contract_name,
                        init_code,
                        creator_code_addresses,
                    })
                })
            })
            .collect()
    }
}

#[derive(Serialize)]
#[serde(bound = "")]
struct JsonResult<'a, N: Network> {
    logs: Vec<String>,
    returns: &'a HashMap<String, NestedValue>,
    #[serde(flatten)]
    result: &'a ScriptResult<N>,
}

#[derive(Clone, Debug)]
pub struct ScriptConfig<FEN: FoundryEvmNetwork> {
    pub config: Config,
    pub evm_opts: EvmOpts,
    /// Executor construction selected by concrete network dispatch.
    pub executor_builder: ExecutorBuilder<FEN>,
    /// Exact network hardfork selected for script execution.
    pub hardfork: Option<FoundryHardfork>,
    /// Source chain used for trace decoding and external identifiers.
    pub source_chain_id: Option<u64>,
    pub sender_nonce: u64,
    sender_nonce_override: Option<u64>,
    resolved_fork: Option<ResolvedFork>,
    /// Backends keyed by their complete resolved fork context.
    backends: HashMap<ResolvedFork, Backend<FEN>>,
    /// Whether to batch all broadcast transactions into a single Tempo batch transaction.
    pub batch: bool,
    /// Tempo transaction options applied to broadcast transactions.
    pub tempo: TempoOpts,
}

async fn resolve_script_fork(
    config: &mut Config,
    evm_opts: &mut EvmOpts,
    active_networks: Option<NetworkConfigs>,
) -> Result<Option<ResolvedFork>> {
    if evm_opts.fork_url.is_none() {
        return Ok(None);
    }
    if evm_opts.fork_endpoint.is_none() {
        evm_opts.infer_network_from_fork().await?;
    }
    if let Some(active_networks) = active_networks
        && !active_networks.supports_fork_source(&evm_opts.networks)
    {
        eyre::bail!(
            "fork network `{}` is incompatible with the active EVM",
            evm_opts.networks.execution_network()
        );
    }
    config.networks = evm_opts.networks;
    if let Some(identity) = evm_opts.fork_endpoint.clone() {
        let network_is_inferred = evm_opts.fork_network_is_inferred;
        evm_opts.expect_fork_endpoint(identity, network_is_inferred);
    }
    evm_opts.resolve_fork().await
}

impl<FEN: FoundryEvmNetwork> ScriptConfig<FEN> {
    pub(crate) async fn new(
        mut config: Config,
        mut evm_opts: EvmOpts,
        executor_builder: ExecutorBuilder<FEN>,
        batch: bool,
        tempo: TempoOpts,
        sender_nonce_override: Option<u64>,
    ) -> Result<Self> {
        // Linking happens before runner construction, so resolve the fork context now and reuse it
        // for all preflight reads and environment construction.
        let resolved_fork = resolve_script_fork(&mut config, &mut evm_opts, None).await?;
        let sender_nonce = if let Some(sender_nonce) = sender_nonce_override {
            sender_nonce
        } else if evm_opts.fork_url.is_some() {
            let fork = resolved_fork.as_ref().context("fork must be resolved")?;
            next_nonce_resolved(evm_opts.sender, &evm_opts, fork).await?
        } else {
            // dapptools compatibility
            1
        };

        Ok(Self {
            config,
            evm_opts,
            executor_builder,
            hardfork: None,
            source_chain_id: None,
            sender_nonce,
            sender_nonce_override,
            resolved_fork,
            backends: HashMap::default(),
            batch,
            tempo,
        })
    }

    pub async fn update_sender(&mut self, sender: Address) -> Result<()> {
        self.sender_nonce = if let Some(sender_nonce) = self.sender_nonce_override {
            sender_nonce
        } else if self.evm_opts.fork_url.is_some() {
            let fork = self.ensure_resolved_fork().await?.context("fork must be resolved")?;
            next_nonce_resolved(sender, &self.evm_opts, &fork).await?
        } else {
            // dapptools compatibility
            1
        };
        self.evm_opts.sender = sender;
        Ok(())
    }

    /// Returns the resolved fork when it still matches the configured source and selector.
    pub fn resolved_fork(&self) -> Result<Option<&ResolvedFork>> {
        match (&self.evm_opts.fork_url, &self.resolved_fork) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(eyre::eyre!("resolved fork exists without a configured fork")),
            (Some(_), Some(fork)) if self.evm_opts.resolved_fork_matches(fork) => Ok(Some(fork)),
            (Some(_), Some(_)) => {
                Err(eyre::eyre!("resolved fork does not match the configured source and selector"))
            }
            (Some(_), None) => Err(eyre::eyre!("fork must be resolved")),
        }
    }

    fn set_fork_url(&mut self, fork_url: String) {
        self.evm_opts.set_fork_url(fork_url);
    }

    async fn ensure_resolved_fork(&mut self) -> Result<Option<ResolvedFork>> {
        if self.evm_opts.fork_url.is_none() {
            if self.resolved_fork.take().is_some() {
                self.backends.clear();
            }
            return Ok(None);
        }
        if let Some(fork) = &self.resolved_fork
            && self.evm_opts.resolved_fork_matches(fork)
        {
            return Ok(Some(fork.clone()));
        }
        // Script execution was already dispatched to `FEN`; compare the new selection with that
        // execution profile, not with the previous RPC source profile.
        let active_networks = Some(self.config.networks);
        if let Some(fork) = &self.resolved_fork {
            self.evm_opts.invalidate_fork_endpoint_if_source_changed(fork);
        }

        let fork =
            resolve_script_fork(&mut self.config, &mut self.evm_opts, active_networks).await?;
        self.resolved_fork = fork.clone();
        Ok(fork)
    }

    pub(crate) async fn update_tempo_session_sender(
        &mut self,
        wallets: &MultiWalletOpts,
        expected_sender: Option<Address>,
    ) -> Result<()> {
        if let Some(sender) =
            self.tempo.session_sender_for_multi_wallet(wallets, expected_sender)?
        {
            self.update_sender(sender).await?;
        }
        Ok(())
    }

    async fn get_runner_with_cheatcodes(
        &mut self,
        known_contracts: ContractsByArtifact,
        script_wallets: Wallets,
        debug: bool,
        target: ArtifactId,
        restricted: bool,
    ) -> Result<ScriptRunner<FEN>> {
        let mut runner = self
            ._get_runner(Some((known_contracts, script_wallets, target)), debug, restricted)
            .await?;

        // Script execution is synthetic. Keep the Tempo transaction context, but do not charge
        // protocol fees for deploying or calling the local script contract.
        if self.evm_opts.networks.is_tempo() {
            runner.executor.evm_env_mut().cfg_env.disable_fee_charge = true;
        }

        Ok(runner)
    }

    async fn _get_runner(
        &mut self,
        cheats_data: Option<(ContractsByArtifact, Wallets, ArtifactId)>,
        debug: bool,
        restricted: bool,
    ) -> Result<ScriptRunner<FEN>> {
        trace!("preparing script runner");
        let (resolved, evm_env, mut tx_env) = self.resolve_execution_env().await?;

        let db = if self.evm_opts.fork_url.is_some() {
            let resolved = resolved.context("fork must be resolved")?;
            if let Some(backend) = self.backends.get(&resolved) {
                backend.clone()
            } else {
                let fork = self.evm_opts.get_fork_resolved(
                    &self.config,
                    evm_env.cfg_env.chain_id,
                    Some(&resolved),
                );
                let backend = Backend::spawn(fork)?;
                self.backends.insert(resolved, backend.clone());
                backend
            }
        } else {
            // It's only really `None`, when we don't pass any `--fork-url`. And if so, there is
            // no need to cache it, since there won't be any onchain simulation that we'd need
            // to cache the backend for.
            Backend::spawn(None)?
        };

        // We need to enable tracing to decode contract names: local or external.
        let mut builder = self
            .executor_builder
            .clone()
            .inspectors(|stack| {
                stack
                    .logs(self.config.live_logs)
                    .trace_requirements(script_trace_requirements(&self.config, debug))
                    .create2_deployer(self.evm_opts.create2_deployer)
            })
            .gas_limit(self.evm_opts.gas_limit())
            .legacy_assertions(self.config.legacy_assertions);

        if let Some((known_contracts, script_wallets, target)) = cheats_data {
            builder = builder.inspectors(|stack| {
                let mut cheats_config = CheatsConfig::new(
                    &self.config,
                    self.evm_opts.clone(),
                    Some(known_contracts),
                    Some(target),
                    self.batch,
                );
                if restricted {
                    cheats_config.blocked_cheatcodes =
                        library_deployments::rerun_unsafe_cheatcode_selectors();
                }
                stack
                    .cheatcodes(cheats_config.into())
                    .wallets(script_wallets)
                    .enable_isolation(self.evm_opts.isolate)
            });
        }

        // Propagate fee token to the transaction environment so that internal EVM calls
        // (e.g. script deployment, setUp) use the correct fee token for Tempo networks.
        tx_env.set_fee_token(self.tempo.fee_token);

        let mut runner = ScriptRunner::new(
            builder.build(evm_env, tx_env, db, self.evm_opts.networks),
            self.evm_opts.clone(),
        )
        .with_debug_bytecodes(debug);

        if self.sender_nonce_override.is_some() {
            runner.executor.set_nonce(self.evm_opts.sender, self.sender_nonce)?;
        }

        Ok(runner)
    }

    /// Resolves the configured fork and execution spec without constructing a database or runner.
    async fn resolve_execution_env(
        &mut self,
    ) -> Result<(Option<ResolvedFork>, EvmEnvFor<FEN>, TxEnvFor<FEN>)> {
        let resolved = self.ensure_resolved_fork().await?;
        let (mut evm_env, tx_env) =
            self.evm_opts.env_with_resolved_fork::<_, _, TxEnvFor<FEN>>(resolved.as_ref()).await?;
        let fork_context = resolved.as_ref().map(ResolvedFork::context);
        let fork_chain_id = fork_context.map(|context| context.source_chain_id);
        let fork_hardfork = fork_context.and_then(|context| context.hardfork);
        self.source_chain_id = fork_chain_id;
        self.hardfork = resolve_execution_spec(
            &self.config,
            self.evm_opts.networks,
            &mut evm_env,
            ExecutionSpecContext::local_or_fork(fork_chain_id, fork_hardfork),
            None,
            None,
        );
        Ok((resolved, evm_env, tx_env))
    }
}

const fn script_trace_requirements(config: &Config, debug: bool) -> TraceRequirements {
    TraceRequirements::none()
        .with_calls(true)
        .with_debug(debug)
        .with_verbosity(config.tracing.verbosity)
        .with_decode_internal(if config.tracing.decode_internal {
            InternalTraceMode::Full
        } else {
            InternalTraceMode::None
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_chains::NamedChain;
    use alloy_eips::BlockId;
    use alloy_network::Ethereum;
    use alloy_primitives::{B256, address};
    use alloy_provider::Provider as _;
    use alloy_rpc_types::TransactionRequest;
    use alloy_signer::SignerSync as _;
    use anvil::{NodeConfig, spawn};
    use foundry_cli::opts::TEMPO_SESSION_ID_ENV;
    use foundry_common::tempo::{
        GeneratedSessionKey, SessionAuthorizationRequest, SessionEntry, TEMPO_HOME_ENV,
        upsert_session_entry,
    };
    use foundry_config::UnresolvedEnvVarError;
    use foundry_evm::{
        revm::context::Block as _,
        traces::{
            CallKind, CallTrace, CallTraceArena, CallTraceNode, SparsedTraceArena, TraceKind,
        },
    };
    use semver::Version;
    use std::{fs, num::NonZeroU64, sync::LazyLock};
    use tempfile::tempdir;
    use tokio::sync::{Mutex, MutexGuard};

    const SESSION_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";
    const SESSION_ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const SESSION_ID_HEX: &str =
        "0x1111111111111111111111111111111111111111111111111111111111111111";
    const SESSION_ROOT_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    static TEMPO_HOME_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn assert_missing_orphaned_block(error: &eyre::Report) {
        let message = format!("{error:#}").to_ascii_lowercase();
        assert!(
            message.contains("failed to get block hash")
                || message.contains("not found")
                || message.contains("unknown block"),
            "unexpected orphaned block lookup error: {error:#}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_fork_context_is_re_resolved_for_a_new_rpc() {
        let (api_a, handle_a) = spawn(NodeConfig::test()).await;
        let (api_b, handle_b) = spawn(NodeConfig::test()).await;
        api_a.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        api_b.anvil_mine(Some(U256::from(3)), None).await.unwrap();

        let evm_opts = EvmOpts {
            fork_url: Some(handle_a.http_endpoint()),
            sender: handle_a.dev_accounts().next().unwrap(),
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        let first = config.resolved_fork.clone().unwrap();
        assert_eq!(first.number(), 1);
        assert_eq!(config.evm_opts.fork_block_number, None);
        assert!(config.evm_opts.resolved_fork_matches(&first));

        config._get_runner(None, false, false).await.unwrap();

        config.set_fork_url(handle_b.http_endpoint());
        let second = config.ensure_resolved_fork().await.unwrap().unwrap();

        assert_eq!(second.number(), 3);
        assert_eq!(config.evm_opts.fork_block_number, None);
        assert!(config.evm_opts.resolved_fork_matches(&second));
        assert_ne!(first, second);
    }

    #[cfg(feature = "monad")]
    #[tokio::test(flavor = "multi_thread")]
    async fn script_explicit_network_is_preserved_for_a_new_rpc() {
        let (api_a, handle_a) = spawn(NodeConfig::test_monad()).await;
        let (api_b, handle_b) = spawn(NodeConfig::test_monad()).await;
        api_a.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        api_b.anvil_mine(Some(U256::from(3)), None).await.unwrap();

        let evm_opts = EvmOpts {
            fork_url: Some(handle_a.http_endpoint()),
            sender: handle_a.dev_accounts().next().unwrap(),
            networks: NetworkConfigs::with_ethereum(),
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(config.evm_opts.networks, NetworkConfigs::with_ethereum());

        config.set_fork_url(handle_b.http_endpoint());
        let second = config.ensure_resolved_fork().await.unwrap().unwrap();

        assert_eq!(second.number(), 3);
        assert_eq!(second.context().network_profile, NetworkConfigs::with_monad());
        assert_eq!(config.evm_opts.networks, NetworkConfigs::with_ethereum());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_explicit_fork_block_is_preserved_for_a_new_rpc() {
        let (api_a, handle_a) = spawn(NodeConfig::test()).await;
        let (api_b, handle_b) = spawn(NodeConfig::test()).await;
        api_a.anvil_mine(Some(U256::from(2)), None).await.unwrap();
        api_b.anvil_mine(Some(U256::from(4)), None).await.unwrap();

        let evm_opts = EvmOpts {
            fork_url: Some(handle_a.http_endpoint()),
            fork_block_number: Some(1),
            sender: handle_a.dev_accounts().next().unwrap(),
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        config._get_runner(None, false, false).await.unwrap();

        config.set_fork_url(handle_b.http_endpoint());
        let context = config.ensure_resolved_fork().await.unwrap().unwrap();

        assert_eq!(context.number(), 1);
        assert_eq!(config.evm_opts.fork_block_number, Some(1));
        assert!(config.evm_opts.resolved_fork_matches(&context));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_explicit_fork_block_can_equal_previous_latest() {
        let (api_a, handle_a) = spawn(NodeConfig::test()).await;
        let (api_b, handle_b) = spawn(NodeConfig::test()).await;
        api_a.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        api_b.anvil_mine(Some(U256::from(1)), None).await.unwrap();

        let evm_opts = EvmOpts {
            fork_url: Some(handle_a.http_endpoint()),
            sender: handle_a.dev_accounts().next().unwrap(),
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        let latest = config.resolved_fork.clone().unwrap();
        assert_eq!(config.evm_opts.fork_block_number, None);
        config._get_runner(None, false, false).await.unwrap();
        assert!(config.backends.contains_key(&latest));

        config.set_fork_url(handle_b.http_endpoint());
        config.evm_opts.fork_block_number = Some(1);
        let explicit = config.ensure_resolved_fork().await.unwrap().unwrap();

        assert_eq!(explicit.number(), 1);
        assert_eq!(config.evm_opts.fork_block_number, Some(1));
        assert!(config.evm_opts.resolved_fork_matches(&explicit));
        assert_ne!(latest, explicit);
        assert!(config.backends.contains_key(&latest));
        assert!(!config.backends.contains_key(&explicit));

        config._get_runner(None, false, false).await.unwrap();
        assert!(config.backends.contains_key(&latest));
        assert!(config.backends.contains_key(&explicit));
        assert_eq!(config.backends.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_fork_cache_distinguishes_same_height_contexts() {
        let (api_a, handle_a) = spawn(NodeConfig::test()).await;
        let (api_b, handle_b) = spawn(NodeConfig::test()).await;
        let state_address = address!("0000000000000000000000000000000000001337");
        let balance_a = U256::from(1);
        let balance_b = U256::from(2);
        api_a.anvil_set_balance(state_address, balance_a).await.unwrap();
        api_b.anvil_set_balance(state_address, balance_b).await.unwrap();
        let original_prevrandao = B256::with_last_byte(0x42);
        api_a.anvil_set_next_block_prevrandao(original_prevrandao).await.unwrap();
        api_a.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        api_b.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        let url_a = handle_a.http_endpoint();
        let url_b = handle_b.http_endpoint();
        let sender = handle_a.dev_accounts().next().unwrap();

        let evm_opts = EvmOpts { fork_url: Some(url_a.clone()), sender, ..Default::default() };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        let runner_a = config._get_runner(None, false, false).await.unwrap();
        assert_eq!(runner_a.executor.get_balance(state_address).unwrap(), balance_a);
        let first_a = config.resolved_fork().unwrap().unwrap().clone();
        assert!(config.backends.contains_key(&first_a));

        config.set_fork_url(url_b.clone());
        let runner_b = config._get_runner(None, false, false).await.unwrap();
        assert_eq!(runner_b.executor.get_balance(state_address).unwrap(), balance_b);
        let context_b = config.resolved_fork().unwrap().unwrap().clone();
        assert_eq!(context_b.number(), 1);
        assert!(config.backends.contains_key(&first_a));
        assert!(config.backends.contains_key(&context_b));
        assert_eq!(config.backends.len(), 2);

        let provider_a = handle_a.http_provider();
        provider_a
            .raw_request::<_, ()>("anvil_reorg".into(), (1_u64, Vec::<serde_json::Value>::new()))
            .await
            .unwrap();
        let replacement = provider_a.get_block_by_number(1.into()).await.unwrap().unwrap();
        assert_eq!(replacement.header.number, first_a.number());
        assert_ne!(replacement.header.hash, first_a.hash());
        assert_ne!(replacement.header.mix_hash, Some(original_prevrandao));

        config.set_fork_url(url_a.clone());
        let second_a = config.ensure_resolved_fork().await.unwrap().unwrap();
        assert_eq!(second_a.number(), first_a.number());
        assert_eq!(second_a.hash(), replacement.header.hash);
        assert_ne!(second_a.hash(), first_a.hash());
        assert!(config.backends.contains_key(&first_a));
        assert!(config.backends.contains_key(&context_b));
        assert!(!config.backends.contains_key(&second_a));
        assert_eq!(config.backends.len(), 2);

        let runner = config._get_runner(None, false, false).await.unwrap();
        assert!(config.backends.contains_key(&second_a));
        assert_eq!(config.backends.len(), 3);
        assert_eq!(runner.executor.get_balance(state_address).unwrap(), balance_a);
        assert_eq!(runner.executor.evm_env().block_env.prevrandao(), replacement.header.mix_hash);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_same_endpoint_does_not_advance_resolved_fork() {
        let (api, handle) = spawn(NodeConfig::test()).await;
        api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        let url = handle.http_endpoint();
        let evm_opts = EvmOpts {
            fork_url: Some(url.clone()),
            sender: handle.dev_accounts().next().unwrap(),
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        let resolved = config.resolved_fork.clone().unwrap();
        config._get_runner(None, false, false).await.unwrap();

        api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        config.set_fork_url(url.clone());
        let runner = config._get_runner(None, false, false).await.unwrap();

        assert_eq!(config.resolved_fork.as_ref(), Some(&resolved));
        assert!(config.backends.contains_key(&resolved));
        assert_eq!(config.backends.len(), 1);
        assert_eq!(runner.executor.evm_env().block_env.number(), U256::from(1));
        assert_eq!(config.evm_opts.fork_block_number, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_fork_changed_headers_replace_cached_backend() {
        let (api, handle) = spawn(NodeConfig::test()).await;
        api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        let url = handle.http_endpoint();
        let evm_opts = EvmOpts {
            fork_url: Some(url.clone()),
            sender: handle.dev_accounts().next().unwrap(),
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        config._get_runner(None, false, false).await.unwrap();
        let without_headers = config.resolved_fork().unwrap().unwrap().clone();
        assert!(config.backends.contains_key(&without_headers));

        config.evm_opts.fork_headers = Some(vec!["x-foundry-test: resolved-fork".to_string()]);
        let with_headers = config.ensure_resolved_fork().await.unwrap().unwrap();
        assert!(config.backends.contains_key(&without_headers));
        assert!(!config.backends.contains_key(&with_headers));
        assert_eq!(config.backends.len(), 1);
        config._get_runner(None, false, false).await.unwrap();

        assert_ne!(without_headers, with_headers);
        assert!(config.evm_opts.resolved_fork_matches(&with_headers));
        assert!(config.backends.contains_key(&without_headers));
        assert!(config.backends.contains_key(&with_headers));
        assert_eq!(config.backends.len(), 2);
    }

    async fn assert_same_url_fork_auth_change_re_resolves(
        configure_initial: impl FnOnce(&mut EvmOpts),
        configure_changed: impl FnOnce(&mut EvmOpts),
    ) {
        let (api, handle) = spawn(NodeConfig::test()).await;
        let mut evm_opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            fork_block_number: Some(0),
            networks: NetworkConfigs::with_ethereum(),
            ..Default::default()
        };
        evm_opts.env.chain_id = Some(42);
        configure_initial(&mut evm_opts);

        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        config._get_runner(None, false, false).await.unwrap();
        let first = config.resolved_fork().unwrap().unwrap().clone();
        let first_instance = first.context().instance_id.unwrap();
        assert_eq!(config.backends.len(), 1);

        api.anvil_reset(None).await.unwrap();
        assert_ne!(api.instance_id(), first_instance);
        configure_changed(&mut config.evm_opts);

        let second = config.ensure_resolved_fork().await.unwrap().unwrap();
        assert_ne!(second.context().instance_id, Some(first_instance));
        assert_ne!(second, first);
        assert_eq!(config.evm_opts.fork_block_number, Some(0));
        assert_eq!(config.evm_opts.networks, NetworkConfigs::with_ethereum());
        assert_eq!(config.evm_opts.env.chain_id, Some(42));
        assert!(config.backends.contains_key(&first));
        assert!(!config.backends.contains_key(&second));

        config._get_runner(None, false, false).await.unwrap();
        assert!(config.backends.contains_key(&first));
        assert!(config.backends.contains_key(&second));
        assert_eq!(config.backends.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_same_url_rpc_headers_change_adopts_new_fork_identity() {
        assert_same_url_fork_auth_change_re_resolves(
            |evm_opts| evm_opts.rpc_headers = Some(vec!["x-fork-source: first".to_string()]),
            |evm_opts| evm_opts.rpc_headers = Some(vec!["x-fork-source: second".to_string()]),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_same_url_jwt_change_adopts_new_fork_identity() {
        const FIRST_JWT: &str = "5c43996d0d150a81f06ae452fce38120d97a4156650aec7487b3384bfe32edae";
        const SECOND_JWT: &str = "cabee703106087906e50f3e75a6ddbab60809f980511d1d1548d449d52220795";

        assert_same_url_fork_auth_change_re_resolves(
            |evm_opts| evm_opts.rpc_jwt = Some(FIRST_JWT.to_string()),
            |evm_opts| evm_opts.rpc_jwt = Some(SECOND_JWT.to_string()),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_runner_env_revalidates_resolved_fork_hash() {
        let (api, handle) = spawn(NodeConfig::test()).await;
        let prevrandao = B256::with_last_byte(0x42);
        api.anvil_set_next_block_prevrandao(prevrandao).await.unwrap();
        api.anvil_mine(Some(U256::from(1)), None).await.unwrap();

        let evm_opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            sender: handle.dev_accounts().next().unwrap(),
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        assert!(
            config
                .evm_opts
                .can_use_create2_deployer_resolved(config.resolved_fork().unwrap())
                .await
                .unwrap()
        );
        config._get_runner(None, false, false).await.unwrap();
        assert_eq!(config.backends.len(), 1);

        let provider = handle.http_provider();
        let pinned = config.resolved_fork.clone().unwrap();
        provider
            .raw_request::<_, ()>("anvil_reorg".into(), (1_u64, Vec::<serde_json::Value>::new()))
            .await
            .unwrap();
        let replacement = provider.get_block_by_number(1.into()).await.unwrap().unwrap();
        assert_ne!(replacement.header.hash, pinned.hash());
        assert_ne!(replacement.header.mix_hash, Some(prevrandao));

        // The environment is hash-revalidated here. The fork database remains number-pinned;
        // full state and ancestry exactness is tracked in #15897.
        config.set_fork_url(handle.http_endpoint());
        match config._get_runner(None, false, false).await {
            Ok(runner) => {
                assert_eq!(runner.executor.evm_env().block_env.prevrandao(), Some(prevrandao));
            }
            Err(error) => assert_missing_orphaned_block(&error),
        }
        assert_eq!(config.resolved_fork.as_ref(), Some(&pinned));
    }

    #[test]
    fn script_trace_requirements_honor_tracing_verbosity() {
        let mut config = Config::default();
        config.tracing.verbosity = 5;

        let tracing = script_trace_requirements(&config, false).into_config().unwrap();
        assert!(tracing.record_state_diff);
    }

    #[test]
    fn created_contracts_include_successful_creator_code_ancestry() {
        let root = Address::repeat_byte(0x11);
        let implementation = Address::repeat_byte(0x22);
        let helper = Address::repeat_byte(0x33);
        let created = Address::repeat_byte(0x44);
        let init_code = Bytes::from_static(&[0x60, 0x00]);
        let mut arena = CallTraceArena::default();
        arena.nodes_mut()[0].trace =
            CallTrace { success: true, address: root, kind: CallKind::Call, ..Default::default() };
        arena.nodes_mut().extend([
            CallTraceNode {
                parent: Some(0),
                idx: 1,
                trace: CallTrace {
                    success: true,
                    address: implementation,
                    kind: CallKind::DelegateCall,
                    ..Default::default()
                },
                ..Default::default()
            },
            CallTraceNode {
                parent: Some(1),
                idx: 2,
                trace: CallTrace {
                    success: true,
                    address: helper,
                    kind: CallKind::Call,
                    ..Default::default()
                },
                ..Default::default()
            },
            CallTraceNode {
                parent: Some(2),
                idx: 3,
                trace: CallTrace {
                    success: true,
                    address: created,
                    kind: CallKind::Create2,
                    data: init_code.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
        ]);
        let result = ScriptResult::<Ethereum> {
            traces: vec![(
                TraceKind::Execution,
                SparsedTraceArena {
                    arena,
                    ignored: Default::default(),
                    diagnostics: Default::default(),
                },
            )],
            ..Default::default()
        };

        let contracts = result.get_created_contracts(&ContractsByArtifact::default());
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].address, created);
        assert_eq!(contracts[0].init_code, init_code);
        assert_eq!(contracts[0].creator_code_addresses, [helper, implementation, root]);
    }

    #[test]
    fn created_contracts_exclude_creations_rolled_back_by_an_ancestor() {
        let mut arena = CallTraceArena::default();
        arena.nodes_mut()[0].trace = CallTrace { success: true, ..Default::default() };
        arena.nodes_mut().extend([
            CallTraceNode {
                parent: Some(0),
                idx: 1,
                trace: CallTrace { success: false, kind: CallKind::Call, ..Default::default() },
                ..Default::default()
            },
            CallTraceNode {
                parent: Some(1),
                idx: 2,
                trace: CallTrace {
                    success: true,
                    kind: CallKind::Create,
                    data: Bytes::from_static(&[0x60, 0x00]),
                    ..Default::default()
                },
                ..Default::default()
            },
        ]);
        let result = ScriptResult::<Ethereum> {
            traces: vec![(
                TraceKind::Execution,
                SparsedTraceArena {
                    arena,
                    ignored: Default::default(),
                    diagnostics: Default::default(),
                },
            )],
            ..Default::default()
        };

        assert!(result.get_created_contracts(&ContractsByArtifact::default()).is_empty());
    }

    fn active_session_entry(
        session_id: B256,
        root_account: Address,
        chain_id: u64,
    ) -> SessionEntry {
        let foundry_wallets::WalletSigner::Local(root) =
            foundry_wallets::utils::create_private_key_signer(SESSION_ROOT_PRIVATE_KEY).unwrap()
        else {
            unreachable!("a raw private key always creates a local signer")
        };
        assert_eq!(root.address(), root_account);
        let key = GeneratedSessionKey::from_private_key(SESSION_PRIVATE_KEY).unwrap();
        let prepared = SessionAuthorizationRequest {
            session_id,
            root_account,
            chain_id,
            key_address: key.address(),
            expiry: NonZeroU64::new(u64::MAX).unwrap(),
            scope: vec![tempo_primitives::transaction::CallScope {
                target: Address::repeat_byte(0xaa),
                selector_rules: vec![],
            }],
            spend_limits: vec![],
        }
        .prepare(0)
        .unwrap();
        let signature = root.sign_hash_sync(&prepared.authorization.signature_hash()).unwrap();
        let authorization = prepared
            .authorization
            .clone()
            .into_signed(tempo_primitives::transaction::PrimitiveSignature::Secp256k1(signature));
        prepared.into_active_entry(key, &authorization).unwrap()
    }

    struct TempoHomeGuard {
        _guard: MutexGuard<'static, ()>,
    }

    impl TempoHomeGuard {
        async fn set(path: &std::path::Path) -> Self {
            let guard = TEMPO_HOME_LOCK.lock().await;
            // SAFETY: test-only environment override for Tempo local state.
            unsafe {
                std::env::remove_var(TEMPO_SESSION_ID_ENV);
                std::env::set_var(TEMPO_HOME_ENV, path);
            }
            Self { _guard: guard }
        }
    }

    impl Drop for TempoHomeGuard {
        fn drop(&mut self) {
            // SAFETY: restore process environment after the critical section.
            unsafe {
                std::env::remove_var(TEMPO_HOME_ENV);
                std::env::remove_var(TEMPO_SESSION_ID_ENV);
            }
        }
    }

    fn session_root() -> Address {
        SESSION_ROOT_ADDRESS.parse().unwrap()
    }

    #[test]
    fn can_parse_sig() {
        let sig = "0x522bb704000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfFFb92266";
        let args = ScriptArgs::parse_from(["foundry-cli", "Contract.sol", "--sig", sig]);
        assert_eq!(args.sig, sig);
    }

    #[test]
    fn parses_confirmations() {
        let args = ScriptArgs::parse_from(["foundry-cli", "Contract.sol"]);
        assert_eq!(args.confirmations, DEFAULT_CONFIRMATIONS);

        let args = ScriptArgs::parse_from(["foundry-cli", "Contract.sol", "--confirmations", "6"]);
        assert_eq!(args.confirmations, 6);

        let err =
            ScriptArgs::try_parse_from(["foundry-cli", "Contract.sol", "--confirmations", "0"])
                .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn verify_external_requires_verify() {
        let err = ScriptArgs::try_parse_from(["foundry-cli", "Contract.sol", "--verify-external"])
            .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);

        let args = ScriptArgs::try_parse_from([
            "foundry-cli",
            "Contract.sol",
            "--broadcast",
            "--verify",
            "--verify-external",
        ])
        .unwrap();
        assert!(args.verify_external);
    }

    #[test]
    fn verify_external_requires_simulation_and_online_mode() {
        for conflicting_arg in ["--skip-simulation", "--offline"] {
            let err = ScriptArgs::try_parse_from([
                "foundry-cli",
                "Contract.sol",
                "--broadcast",
                "--verify",
                "--verify-external",
                conflicting_arg,
            ])
            .unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn rejects_max_sender_nonce() {
        let max_valid_nonce = (u64::MAX - 1).to_string();
        let args = ScriptArgs::try_parse_from([
            "foundry-cli",
            "Contract.sol",
            "--sender-nonce",
            &max_valid_nonce,
        ])
        .unwrap();
        assert_eq!(args.sender_nonce, Some(u64::MAX - 1));

        let max_nonce = u64::MAX.to_string();
        let err = ScriptArgs::try_parse_from([
            "foundry-cli",
            "Contract.sol",
            "--sender-nonce",
            &max_nonce,
        ])
        .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_sender_fork_does_not_read_nonce_from_replacement_block() {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let accounts = handle.dev_wallets().collect::<Vec<_>>();
        let original_sender = accounts[0].address();
        let replacement_sender = accounts[1].address();
        let provider = handle.http_provider();
        let tx = TransactionRequest::default()
            .from(replacement_sender)
            .to(original_sender)
            .value(U256::from(1));
        provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

        let evm_opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            sender: original_sender,
            ..Default::default()
        };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();
        let resolved = config.resolved_fork.clone().unwrap();
        assert_eq!(config.evm_opts.fork_block_number, None);
        assert_eq!(config.sender_nonce, 0);

        provider
            .raw_request::<_, ()>("anvil_reorg".into(), (1_u64, Vec::<serde_json::Value>::new()))
            .await
            .unwrap();
        assert_eq!(
            provider
                .get_transaction_count(replacement_sender)
                .block_id(BlockId::number(1))
                .await
                .unwrap(),
            0
        );

        match config.update_sender(replacement_sender).await {
            Ok(()) => assert_eq!(config.sender_nonce, 1),
            Err(error) => assert_missing_orphaned_block(&error),
        }
        assert_eq!(config.resolved_fork.as_ref(), Some(&resolved));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sender_nonce_override_does_not_refresh_fork() {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let sender = handle.dev_accounts().next().unwrap();
        let evm_opts =
            EvmOpts { fork_url: Some(handle.http_endpoint()), sender, ..Default::default() };
        let mut config = ScriptConfig::<EthEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<EthEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            Some(7),
        )
        .await
        .unwrap();

        config.set_fork_url("http://127.0.0.1:1".to_string());
        config.update_sender(Address::with_last_byte(0x42)).await.unwrap();
        assert_eq!(config.sender_nonce, 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn script_runner_preserves_nested_fork_source_chain() {
        let (_origin_api, origin_handle) = spawn(
            NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::MonadTestnet as u64))
                .with_hardfork(Some(foundry_evm::hardforks::MonadHardfork::MonadNine.into())),
        )
        .await;
        let (_fork_api, fork_handle) = spawn(
            NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::Mainnet as u64))
                .with_no_storage_caching(true)
                .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
                .with_fork_block_number(Some(0u64)),
        )
        .await;
        let mut evm_opts =
            EvmOpts { fork_url: Some(fork_handle.http_endpoint()), ..Default::default() };
        evm_opts.infer_network_from_fork().await.unwrap();
        let config = Config { networks: evm_opts.networks, ..Default::default() };
        let mut script = ScriptConfig::<MonadEvmNetwork>::new(
            config,
            evm_opts,
            ExecutorBuilder::<MonadEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();

        let _runner = script._get_runner(None, false, false).await.unwrap();

        assert_eq!(script.source_chain_id, Some(NamedChain::MonadTestnet as u64));
        assert_eq!(
            script.hardfork,
            Some(FoundryHardfork::Monad(foundry_evm::hardforks::MonadHardfork::MonadNine))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tempo_runner_fee_charge_matches_execution_context() {
        let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
        let networks = NetworkConfigs::with_tempo();
        let evm_opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            sender: handle.dev_accounts().next().unwrap(),
            networks,
            ..Default::default()
        };
        let config = Config { networks, ..Default::default() };
        let mut script = ScriptConfig::<TempoEvmNetwork>::new(
            config,
            evm_opts,
            ExecutorBuilder::<TempoEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            None,
        )
        .await
        .unwrap();

        let rpc_runner = script._get_runner(None, false, false).await.unwrap();
        assert!(!rpc_runner.executor.evm_env().cfg_env.disable_fee_charge);

        let target = ArtifactId {
            path: PathBuf::from("Script.json"),
            name: "Script".to_string(),
            source: PathBuf::from("Script.sol"),
            version: Version::new(0, 8, 30),
            build_id: String::new(),
            profile: "default".to_string(),
        };
        let synthetic_runner = script
            .get_runner_with_cheatcodes(
                ContractsByArtifact::default(),
                Wallets::new(Default::default(), None),
                false,
                target,
                false,
            )
            .await
            .unwrap();
        assert!(synthetic_runner.executor.evm_env().cfg_env.disable_fee_charge);
    }

    #[test]
    fn can_parse_shared_tempo_opts() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--tempo.fee-token",
            "1",
            "--tempo.expires",
            "10",
        ]);

        assert_eq!(
            args.tempo.fee_token,
            Some(address!("0x20C0000000000000000000000000000000000001"))
        );
        assert_eq!(args.tempo.expires, Some(10));
    }

    #[test]
    fn can_parse_sponsor_tempo_opts() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--tempo.sponsor",
            SESSION_ROOT_ADDRESS,
            "--tempo.sponsor-signer",
            "env://TEMPO_SPONSOR_PK",
        ]);

        assert_eq!(args.tempo.sponsor, Some(session_root()));
        assert_eq!(args.tempo.sponsor_signer.as_deref(), Some("env://TEMPO_SPONSOR_PK"));
    }

    #[test]
    fn can_parse_full_tempo_opts() {
        let args =
            ScriptArgs::parse_from(["foundry-cli", "Contract.sol", "--tempo.nonce-key", "1"]);

        assert_eq!(args.tempo.nonce_key, Some(U256::from(1)));
    }

    #[test]
    fn can_parse_tempo_session_opt() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--tempo.session",
            SESSION_ID_HEX,
        ]);

        assert_eq!(args.tempo.session, Some(B256::from([0x11; 32])),);
    }

    #[tokio::test]
    async fn tempo_session_sets_script_sender_to_root_account() {
        let temp = tempdir().unwrap();
        let session_id = B256::from([0x22; 32]);
        let root = session_root();
        let chain_id = foundry_common::DEV_CHAIN_ID;

        let _guard = TempoHomeGuard::set(temp.path()).await;
        upsert_session_entry(active_session_entry(session_id, root, chain_id)).unwrap();

        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--tempo.session",
            &format!("{session_id:?}"),
        ]);
        let evm_opts = EvmOpts {
            networks: NetworkConfigs::with_tempo(),
            env: foundry_evm::opts::Env { chain_id: Some(chain_id), ..Default::default() },
            ..Default::default()
        };

        let state = args
            .preprocess::<TempoEvmNetwork>(
                Config::default(),
                evm_opts,
                ExecutorBuilder::<TempoEvmNetwork>::new(),
            )
            .await
            .unwrap();
        assert_eq!(state.script_config.evm_opts.sender, root);
    }

    #[tokio::test]
    async fn tempo_session_resume_multi_defers_session_sender_until_reexecution() {
        let temp = tempdir().unwrap();
        let session_id = B256::from([0x55; 32]);
        let root = session_root();
        let chain_id = 4217;

        let _guard = TempoHomeGuard::set(temp.path()).await;
        upsert_session_entry(active_session_entry(session_id, root, chain_id)).unwrap();

        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--resume",
            "--multi",
            "--tempo.session",
            &format!("{session_id:?}"),
        ]);
        let evm_opts = EvmOpts { networks: NetworkConfigs::with_tempo(), ..Default::default() };

        let state = args
            .preprocess::<TempoEvmNetwork>(
                Config::default(),
                evm_opts,
                ExecutorBuilder::<TempoEvmNetwork>::new(),
            )
            .await
            .unwrap();
        assert_ne!(state.script_config.evm_opts.sender, root);
    }

    #[tokio::test]
    async fn tempo_session_resume_defers_session_sender_until_reexecution() {
        let temp = tempdir().unwrap();
        let session_id = B256::from([0x77; 32]);
        let root = session_root();
        let chain_id = 4217;

        let _guard = TempoHomeGuard::set(temp.path()).await;
        upsert_session_entry(active_session_entry(session_id, root, chain_id)).unwrap();

        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--resume",
            "--tempo.session",
            &format!("{session_id:?}"),
        ]);
        let evm_opts = EvmOpts { networks: NetworkConfigs::with_tempo(), ..Default::default() };

        let state = args
            .preprocess::<TempoEvmNetwork>(
                Config::default(),
                evm_opts,
                ExecutorBuilder::<TempoEvmNetwork>::new(),
            )
            .await
            .unwrap();
        assert_ne!(state.script_config.evm_opts.sender, root);
    }

    #[tokio::test]
    async fn tempo_session_non_resume_multi_sets_sender_without_chain_validation() {
        let temp = tempdir().unwrap();
        let session_id = B256::from([0x66; 32]);
        let root = session_root();
        let chain_id = 4217;

        let _guard = TempoHomeGuard::set(temp.path()).await;
        upsert_session_entry(active_session_entry(session_id, root, chain_id)).unwrap();

        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--multi",
            "--tempo.session",
            &format!("{session_id:?}"),
        ]);
        let evm_opts = EvmOpts { networks: NetworkConfigs::with_tempo(), ..Default::default() };

        let state = args
            .preprocess::<TempoEvmNetwork>(
                Config::default(),
                evm_opts,
                ExecutorBuilder::<TempoEvmNetwork>::new(),
            )
            .await
            .unwrap();
        assert_eq!(state.script_config.evm_opts.sender, root);
    }

    #[tokio::test]
    async fn tempo_session_initial_broadcast_sets_sender_without_chain_validation() {
        let temp = tempdir().unwrap();
        let session_id = B256::from([0x88; 32]);
        let root = session_root();
        let chain_id = 4217;

        let _guard = TempoHomeGuard::set(temp.path()).await;
        upsert_session_entry(active_session_entry(session_id, root, chain_id)).unwrap();

        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--broadcast",
            "--tempo.session",
            &format!("{session_id:?}"),
        ]);
        let evm_opts = EvmOpts { networks: NetworkConfigs::with_tempo(), ..Default::default() };

        let state = args
            .preprocess::<TempoEvmNetwork>(
                Config::default(),
                evm_opts,
                ExecutorBuilder::<TempoEvmNetwork>::new(),
            )
            .await
            .unwrap();
        assert_eq!(state.script_config.evm_opts.sender, root);
    }

    #[tokio::test]
    async fn tempo_session_env_selects_tempo_network() {
        let temp = tempdir().unwrap();
        let _guard = TempoHomeGuard::set(temp.path()).await;
        let session_id = B256::from([0x44; 32]);
        // SAFETY: serialized by TempoHomeGuard.
        unsafe { std::env::set_var(TEMPO_SESSION_ID_ENV, format!("{session_id:?}")) };

        let args = ScriptArgs::parse_from(["foundry-cli", "Contract.sol"]);
        let (_, evm_opts) = args.resolved_evm_opts().await.unwrap();

        assert!(evm_opts.networks.is_tempo());
    }

    #[tokio::test]
    async fn tempo_session_rejects_explicit_script_wallet_signer() {
        let temp = tempdir().unwrap();
        let session_id = B256::from([0x33; 32]);
        let root = session_root();
        let chain_id = foundry_common::DEV_CHAIN_ID;

        let _guard = TempoHomeGuard::set(temp.path()).await;
        upsert_session_entry(active_session_entry(session_id, root, chain_id)).unwrap();

        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--tempo.session",
            &format!("{session_id:?}"),
            "--private-key",
            SESSION_PRIVATE_KEY,
        ]);
        let evm_opts = EvmOpts {
            networks: NetworkConfigs::with_tempo(),
            env: foundry_evm::opts::Env { chain_id: Some(chain_id), ..Default::default() },
            ..Default::default()
        };

        let err = match args
            .preprocess::<TempoEvmNetwork>(
                Config::default(),
                evm_opts,
                ExecutorBuilder::<TempoEvmNetwork>::new(),
            )
            .await
        {
            Ok(_) => panic!("expected --tempo.session with --private-key to fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("explicit wallet signer"), "{err}");
    }

    #[test]
    fn can_parse_unlocked() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--sender",
            "0x4e59b44847b379578588920ca78fbf26c0b4956c",
            "--unlocked",
        ]);
        assert!(args.unlocked);

        let key = U256::ZERO;
        let args = ScriptArgs::try_parse_from([
            "foundry-cli",
            "Contract.sol",
            "--sender",
            "0x4e59b44847b379578588920ca78fbf26c0b4956c",
            "--unlocked",
            "--private-key",
            &key.to_string(),
        ]);
        assert!(args.is_err());
    }

    #[test]
    fn can_merge_script_config() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--etherscan-api-key",
            "goerli",
        ]);
        let config = args.load_config().unwrap();
        assert_eq!(config.etherscan_api_key, Some("goerli".to_string()));
    }

    #[test]
    fn can_disable_code_size_limit() {
        let args =
            ScriptArgs::parse_from(["foundry-cli", "Contract.sol", "--disable-code-size-limit"]);
        assert!(args.disable_code_size_limit);

        let result = ScriptResult::<Ethereum>::default();
        let contracts = ContractsByArtifact::default();
        let create = Address::ZERO;
        assert!(
            args.check_contract_sizes(ContractSizeLimits::default(), &result, &contracts, create)
                .is_ok()
        );
    }

    #[test]
    fn can_parse_verifier_url() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "script",
            "script/Test.s.sol:TestScript",
            "--fork-url",
            "http://localhost:8545",
            "--verifier-url",
            "http://localhost:3000/api/verify",
            "--etherscan-api-key",
            "blacksmith",
            "--broadcast",
            "--verify",
            "-vvvvv",
        ]);
        assert_eq!(
            args.verifier.verifier_url,
            Some("http://localhost:3000/api/verify".to_string())
        );
    }

    #[test]
    fn can_extract_code_size_limit() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "script",
            "script/Test.s.sol:TestScript",
            "--fork-url",
            "http://localhost:8545",
            "--broadcast",
            "--code-size-limit",
            "50000",
        ]);
        assert_eq!(args.evm.env.code_size_limit, Some(50000));
    }

    /// `--code-size-limit` on the CLI should be used by `check_contract_sizes`, not silently
    /// ignored in favour of the foundry.toml value (which defaults to None → EIP-170's 24576).
    #[test]
    fn cli_code_size_limit_is_honoured_by_check() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "script",
            "script/Test.s.sol:TestScript",
            "--code-size-limit",
            "2147483647",
        ]);
        // The CLI flag must land in evm_opts so that the size_limits computation in run() picks
        // it up via `.evm_opts.env.code_size_limit.or(config.code_size_limit)`.
        assert_eq!(args.evm.env.code_size_limit, Some(2147483647));
    }

    #[test]
    #[cfg(feature = "monad")]
    fn contract_size_limits_use_resolved_monad_network() {
        let args =
            ScriptArgs::parse_from(["foundry-cli", "script", "script/Test.s.sol:TestScript"]);
        let evm_opts = EvmOpts { networks: NetworkConfigs::with_monad(), ..Default::default() };

        let limits = args.contract_size_limits::<MonadEvmNetwork>(&Config::default(), &evm_opts);

        assert!(limits.runtime > ContractSizeLimits::default().runtime);
        assert!(limits.initcode > ContractSizeLimits::default().initcode);
    }

    #[test]
    #[cfg(feature = "monad")]
    fn contract_size_limits_prefer_cli_then_config_over_network() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "script",
            "script/Test.s.sol:TestScript",
            "--code-size-limit",
            "64",
        ]);
        let mut config = Config { code_size_limit: Some(128), ..Default::default() };
        let evm_opts = EvmOpts { networks: NetworkConfigs::with_monad(), ..Default::default() };
        assert_eq!(
            args.contract_size_limits::<MonadEvmNetwork>(&config, &evm_opts,),
            ContractSizeLimits::with_runtime_limit(64)
        );

        let args =
            ScriptArgs::parse_from(["foundry-cli", "script", "script/Test.s.sol:TestScript"]);
        config.code_size_limit = Some(128);
        assert_eq!(
            args.contract_size_limits::<MonadEvmNetwork>(&config, &evm_opts,),
            ContractSizeLimits::with_runtime_limit(128)
        );
    }

    #[test]
    fn can_extract_script_etherscan_key() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]
                etherscan_api_key = "amoy"

                [etherscan]
                amoy = { key = "https://etherscan-amoy.com/" }
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--etherscan-api-key",
            "amoy",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let config = args.load_config().unwrap();
        let amoy = config.get_etherscan_api_key(Some(NamedChain::PolygonAmoy.into()));
        assert_eq!(amoy, Some("https://etherscan-amoy.com/".to_string()));
    }

    #[test]
    fn can_extract_script_rpc_alias() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]

                [rpc_endpoints]
                polygonAmoy = "https://polygon-amoy.g.alchemy.com/v2/${_CAN_EXTRACT_RPC_ALIAS}"
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "DeployV1",
            "--rpc-url",
            "polygonAmoy",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let err = args.load_config_and_evm_opts().unwrap_err();

        assert!(err.downcast::<UnresolvedEnvVarError>().is_ok());

        unsafe {
            std::env::set_var("_CAN_EXTRACT_RPC_ALIAS", "123456");
        }
        let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
        assert_eq!(config.eth_rpc_url, Some("polygonAmoy".to_string()));
        assert_eq!(
            evm_opts.fork_url,
            Some("https://polygon-amoy.g.alchemy.com/v2/123456".to_string())
        );
    }

    #[test]
    fn can_extract_script_rpc_and_etherscan_alias() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
            [profile.default]

            [rpc_endpoints]
            amoy = "https://polygon-amoy.g.alchemy.com/v2/${_EXTRACT_RPC_ALIAS}"

            [etherscan]
            amoy = { key = "${_ETHERSCAN_API_KEY}", chain = 80002, url = "https://amoy.polygonscan.com/" }
        "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "DeployV1",
            "--rpc-url",
            "amoy",
            "--etherscan-api-key",
            "amoy",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);
        let err = args.load_config_and_evm_opts().unwrap_err();

        assert!(err.downcast::<UnresolvedEnvVarError>().is_ok());

        unsafe {
            std::env::set_var("_EXTRACT_RPC_ALIAS", "123456");
        }
        unsafe {
            std::env::set_var("_ETHERSCAN_API_KEY", "etherscan_api_key");
        }
        let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
        assert_eq!(config.eth_rpc_url, Some("amoy".to_string()));
        assert_eq!(
            evm_opts.fork_url,
            Some("https://polygon-amoy.g.alchemy.com/v2/123456".to_string())
        );
        let etherscan = config.get_etherscan_api_key(Some(80002u64.into()));
        assert_eq!(etherscan, Some("etherscan_api_key".to_string()));
        let etherscan = config.get_etherscan_api_key(None);
        assert_eq!(etherscan, Some("etherscan_api_key".to_string()));
    }

    #[test]
    fn can_extract_script_rpc_and_sole_etherscan_alias() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]

               [rpc_endpoints]
                amoy = "https://polygon-amoy.g.alchemy.com/v2/${_SOLE_EXTRACT_RPC_ALIAS}"

                [etherscan]
                amoy = { key = "${_SOLE_ETHERSCAN_API_KEY}" }
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "DeployV1",
            "--rpc-url",
            "amoy",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);
        let err = args.load_config_and_evm_opts().unwrap_err();

        assert!(err.downcast::<UnresolvedEnvVarError>().is_ok());

        unsafe {
            std::env::set_var("_SOLE_EXTRACT_RPC_ALIAS", "123456");
        }
        unsafe {
            std::env::set_var("_SOLE_ETHERSCAN_API_KEY", "etherscan_api_key");
        }
        let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
        assert_eq!(
            evm_opts.fork_url,
            Some("https://polygon-amoy.g.alchemy.com/v2/123456".to_string())
        );
        let etherscan = config.get_etherscan_api_key(Some(80002u64.into()));
        assert_eq!(etherscan, Some("etherscan_api_key".to_string()));
        let etherscan = config.get_etherscan_api_key(None);
        assert_eq!(etherscan, Some("etherscan_api_key".to_string()));
    }

    // <https://github.com/foundry-rs/foundry/issues/5923>
    #[test]
    fn test_5923() {
        let args =
            ScriptArgs::parse_from(["foundry-cli", "DeployV1", "--priority-gas-price", "100"]);
        assert!(args.priority_gas_price.is_some());
    }

    #[test]
    fn test_eip1559_fee_estimate() {
        // Defaults to unset (config provides `market`).
        let args = ScriptArgs::parse_from(["foundry-cli", "DeployV1"]);
        assert!(args.eip1559_fee_estimate.is_none());

        let args = ScriptArgs::parse_from(["foundry-cli", "DeployV1", "--estimate", "aggressive"]);
        assert_eq!(args.eip1559_fee_estimate, Some(Eip1559FeeEstimatePreset::Aggressive));
    }

    // <https://github.com/foundry-rs/foundry/issues/5910>
    #[test]
    fn test_5910() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "--broadcast",
            "--with-gas-price",
            "0",
            "SolveTutorial",
        ]);
        assert!(args.with_gas_price.unwrap().is_zero());
    }

    #[test]
    fn test_priority_gas_price_cannot_exceed_gas_price() {
        let args = ScriptArgs::parse_from([
            "foundry-cli",
            "--broadcast",
            "--with-gas-price",
            "100",
            "--priority-gas-price",
            "200",
            "Script",
        ]);
        // priority (200) > max_fee (100) — broadcast should reject this at runtime
        assert!(args.priority_gas_price.unwrap() > args.with_gas_price.unwrap());
    }
}
