use crate::{debug::handle_traces, utils::apply_chain_and_block_specific_env_changes};
use alloy_consensus::Transaction;
use alloy_network::{AnyNetwork, TransactionResponse};
use alloy_primitives::{
    Address, Bytes, U256,
    map::{AddressSet, HashMap},
};
use alloy_provider::Provider;
use alloy_rpc_types::BlockTransactions;
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{TraceResult, init_progress},
};
use foundry_common::{SYSTEM_TRANSACTION_TYPE, is_impersonated_tx, is_known_system_sender, shell};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    Config,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
use foundry_evm::{
    Env,
    core::env::AsEnvMut,
    executors::{EvmError, Executor, TracingExecutor},
    opts::EvmOpts,
    traces::{InternalTraceMode, TraceMode, Traces},
    utils::configure_tx_env,
};
use futures::TryFutureExt;
use revm::DatabaseRef;

/// CLI arguments for `cast run`.
#[derive(Clone, Debug, Parser)]
pub struct RunArgs {
    /// The transaction hash.
    tx_hash: String,

    /// Opens the transaction in the debugger.
    #[arg(long, short)]
    debug: bool,

    /// Whether to identify internal functions in traces.
    #[arg(long)]
    decode_internal: bool,

    /// Defines the depth of a trace
    #[arg(long)]
    trace_depth: Option<usize>,

    /// Print out opcode traces.
    #[arg(long, short)]
    trace_printer: bool,

    /// Executes the transaction only with the state from the previous block.
    ///
    /// May result in different results than the live execution!
    #[arg(long)]
    quick: bool,

    /// Whether to replay system transactions.
    #[arg(long, alias = "sys")]
    replay_system_txes: bool,

    /// Disables the labels in the traces.
    #[arg(long, default_value_t = false)]
    disable_labels: bool,

    /// Label addresses in the trace.
    ///
    /// Example: 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth
    #[arg(long, short)]
    label: Vec<String>,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,

    /// The EVM version to use.
    ///
    /// Overrides the version specified in the config.
    #[arg(long)]
    evm_version: Option<EvmVersion>,

    /// Sets the number of assumed available compute units per second for this provider
    ///
    /// default value: 330
    ///
    /// See also, <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    #[arg(long, alias = "cups", value_name = "CUPS")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    ///
    /// default value: false
    ///
    /// See also, <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    #[arg(long, value_name = "NO_RATE_LIMITS", visible_alias = "no-rpc-rate-limit")]
    pub no_rate_limit: bool,

    /// Use current project artifacts for trace decoding.
    #[arg(long, visible_alias = "la")]
    pub with_local_artifacts: bool,

    /// Disable block gas limit check.
    #[arg(long)]
    pub disable_block_gas_limit: bool,

    /// Enable the tx gas limit checks as imposed by Osaka (EIP-7825).
    #[arg(long)]
    pub enable_tx_gas_limit: bool,
}

impl RunArgs {
    /// Executes the transaction by replaying it
    ///
    /// This replays the entire block the transaction was mined in unless `quick` is set to true
    ///
    /// Note: This executes the transaction(s) as is: Cheatcodes are disabled
    pub async fn run(self) -> Result<()> {
        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::from_provider(figment)?.sanitized();

        let label = self.label;
        let with_local_artifacts = self.with_local_artifacts;
        let debug = self.debug;
        let decode_internal = self.decode_internal;
        let disable_labels = self.disable_labels;
        let compute_units_per_second =
            if self.no_rate_limit { Some(u64::MAX) } else { self.compute_units_per_second };

        let provider = foundry_cli::utils::get_provider_builder(&config)?
            .compute_units_per_second_opt(compute_units_per_second)
            .build()?;

        let tx_hash = self.tx_hash.parse().wrap_err("invalid tx hash")?;
        let tx = provider
            .get_transaction_by_hash(tx_hash)
            .await
            .wrap_err_with(|| format!("tx not found: {tx_hash:?}"))?
            .ok_or_else(|| eyre::eyre!("tx not found: {:?}", tx_hash))?;

        // check if the tx is a system transaction
        if !self.replay_system_txes
            && (is_known_system_sender(tx.from())
                || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE))
        {
            return Err(eyre::eyre!(
                "{:?} is a system transaction.\nReplaying system transactions is currently not supported.",
                tx.tx_hash()
            ));
        }

        let tx_block_number =
            tx.block_number.ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;

        // we need to fork off the parent block
        config.fork_block_number = Some(tx_block_number - 1);

        let create2_deployer = evm_opts.create2_deployer;
        let (block, (mut env, fork, chain, networks)) = tokio::try_join!(
            // fetch the block the transaction was mined in
            provider.get_block(tx_block_number.into()).full().into_future().map_err(Into::into),
            TracingExecutor::get_fork_material(&mut config, evm_opts)
        )?;

        let mut evm_version = self.evm_version;

        env.evm_env.cfg_env.disable_block_gas_limit = self.disable_block_gas_limit;

        // By default do not enforce transaction gas limits imposed by Osaka (EIP-7825).
        // Users can opt-in to enable these limits by setting `enable_tx_gas_limit` to true.
        if !self.enable_tx_gas_limit {
            env.evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        }

        env.evm_env.cfg_env.limit_contract_code_size = None;
        env.evm_env.block_env.number = U256::from(tx_block_number);

        if let Some(block) = &block {
            env.evm_env.block_env.timestamp = U256::from(block.header.timestamp);
            env.evm_env.block_env.beneficiary = block.header.beneficiary;
            env.evm_env.block_env.difficulty = block.header.difficulty;
            env.evm_env.block_env.prevrandao = Some(block.header.mix_hash.unwrap_or_default());
            env.evm_env.block_env.basefee = block.header.base_fee_per_gas.unwrap_or_default();
            env.evm_env.block_env.gas_limit = block.header.gas_limit;

            // TODO: we need a smarter way to map the block to the corresponding evm_version for
            // commonly used chains
            if evm_version.is_none() {
                // if the block has the excess_blob_gas field, we assume it's a Cancun block
                if block.header.excess_blob_gas.is_some() {
                    evm_version = Some(EvmVersion::Prague);
                }
            }
            apply_chain_and_block_specific_env_changes::<AnyNetwork>(
                env.as_env_mut(),
                block,
                config.networks,
            );
        }

        let trace_mode = TraceMode::Call
            .with_debug(self.debug)
            .with_decode_internal(if self.decode_internal {
                InternalTraceMode::Full
            } else {
                InternalTraceMode::None
            })
            .with_state_changes(shell::verbosity() > 4);
        let mut executor = TracingExecutor::new(
            env.clone(),
            fork,
            evm_version,
            trace_mode,
            networks,
            create2_deployer,
            None,
        )?;
        let mut env = Env::new_with_spec_id(
            env.evm_env.cfg_env.clone(),
            env.evm_env.block_env.clone(),
            env.tx.clone(),
            executor.spec_id(),
        );

        // Set the state to the moment right before the transaction
        if !self.quick {
            if !shell::is_json() {
                sh_println!("Executing previous transactions from the block.")?;
            }

            if let Some(block) = block {
                let pb = init_progress(block.transactions.len() as u64, "tx");
                pb.set_position(0);

                let BlockTransactions::Full(ref txs) = block.transactions else {
                    return Err(eyre::eyre!("Could not get block txs"));
                };

                for (index, tx) in txs.iter().enumerate() {
                    // Replay system transactions only if running with `sys` option.
                    // System transactions such as on L2s don't contain any pricing info so it
                    // could cause reverts.
                    if !self.replay_system_txes
                        && (is_known_system_sender(tx.from())
                            || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE))
                    {
                        pb.set_position((index + 1) as u64);
                        continue;
                    }
                    if tx.tx_hash() == tx_hash {
                        break;
                    }

                    configure_tx_env(&mut env.as_env_mut(), &tx.inner);

                    env.evm_env.cfg_env.disable_balance_check = true;

                    if let Some(to) = Transaction::to(tx) {
                        trace!(tx=?tx.tx_hash(),?to, "executing previous call transaction");
                        executor.transact_with_env(env.clone()).wrap_err_with(|| {
                            format!(
                                "Failed to execute transaction: {:?} in block {}",
                                tx.tx_hash(),
                                env.evm_env.block_env.number
                            )
                        })?;
                    } else {
                        trace!(tx=?tx.tx_hash(), "executing previous create transaction");
                        if let Err(error) = executor.deploy_with_env(env.clone(), None) {
                            match error {
                                // Reverted transactions should be skipped
                                EvmError::Execution(_) => (),
                                error => {
                                    return Err(error).wrap_err_with(|| {
                                        format!(
                                            "Failed to deploy transaction: {:?} in block {}",
                                            tx.tx_hash(),
                                            env.evm_env.block_env.number
                                        )
                                    });
                                }
                            }
                        }
                    }

                    pb.set_position((index + 1) as u64);
                }
            }
        }

        // Execute our transaction
        let result = {
            executor.set_trace_printer(self.trace_printer);

            configure_tx_env(&mut env.as_env_mut(), &tx.inner);
            if is_impersonated_tx(tx.inner.inner.inner()) {
                env.evm_env.cfg_env.disable_balance_check = true;
            }

            if let Some(to) = Transaction::to(&tx) {
                trace!(tx=?tx.tx_hash(), to=?to, "executing call transaction");
                TraceResult::try_from(executor.transact_with_env(env))?
            } else {
                trace!(tx=?tx.tx_hash(), "executing create transaction");
                TraceResult::try_from(executor.deploy_with_env(env, None))?
            }
        };

        let contracts_bytecode = fetch_contracts_bytecode_from_trace(&executor, &result)?;
        handle_traces(
            result,
            &config,
            chain,
            &contracts_bytecode,
            label,
            with_local_artifacts,
            debug,
            decode_internal,
            disable_labels,
            self.trace_depth,
        )
        .await?;

        Ok(())
    }
}

pub fn fetch_contracts_bytecode_from_trace(
    executor: &Executor,
    result: &TraceResult,
) -> Result<HashMap<Address, Bytes>> {
    let mut contracts_bytecode = HashMap::default();
    if let Some(ref traces) = result.traces {
        contracts_bytecode.extend(gather_trace_addresses(traces).filter_map(|addr| {
            // All relevant bytecodes should already be cached in the executor.
            let code = executor
                .backend()
                .basic_ref(addr)
                .inspect_err(|e| _ = sh_warn!("Failed to fetch code for {addr}: {e}"))
                .ok()??
                .code?
                .bytes();
            if code.is_empty() {
                return None;
            }
            Some((addr, code))
        }));
    }
    Ok(contracts_bytecode)
}

fn gather_trace_addresses(traces: &Traces) -> impl Iterator<Item = Address> {
    let mut addresses = AddressSet::default();
    for (_, trace) in traces {
        for node in trace.arena.nodes() {
            if !node.trace.address.is_zero() {
                addresses.insert(node.trace.address);
            }
            if !node.trace.caller.is_zero() {
                addresses.insert(node.trace.caller);
            }
        }
    }
    addresses.into_iter()
}

impl figment::Provider for RunArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("RunArgs")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut map = Map::new();

        if let Some(api_key) = &self.etherscan.key {
            map.insert("etherscan_api_key".into(), api_key.as_str().into());
        }

        if let Some(evm_version) = self.evm_version {
            map.insert("evm_version".into(), figment::value::Value::serialize(evm_version)?);
        }

        Ok(Map::from([(Config::selected_profile(), map)]))
    }
}
