use crate::{
    debug::handle_traces,
    utils::{apply_chain_and_block_specific_env_changes, block_env_from_header},
};
use alloy_consensus::{BlockHeader, Transaction, transaction::SignerRecoverable};

use alloy_evm::FromRecoveredTx;
use alloy_network::{BlockResponse, TransactionResponse};
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
use foundry_common::{
    SYSTEM_TRANSACTION_TYPE, is_known_system_sender, provider::ProviderBuilder, shell,
};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    Config,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
use foundry_evm::{
    core::{
        FoundryBlock as _,
        evm::{EthEvmNetwork, FoundryEvmNetwork, OpEvmNetwork, TempoEvmNetwork, TxEnvFor},
    },
    executors::{EvmError, Executor, TracingExecutor},
    hardforks::FoundryHardfork,
    opts::EvmOpts,
    traces::{InternalTraceMode, TraceMode, Traces},
};
use futures::TryFutureExt;
use revm::{DatabaseRef, context::Block};

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
        let mut evm_opts = figment.extract::<EvmOpts>()?;

        // Auto-detect network from fork chain ID when not explicitly configured.
        evm_opts.infer_network_from_fork().await;

        if evm_opts.networks.is_tempo() {
            self.run_with_evm::<TempoEvmNetwork>().await
        } else if evm_opts.networks.is_optimism() {
            self.run_with_evm::<OpEvmNetwork>().await
        } else {
            self.run_with_evm::<EthEvmNetwork>().await
        }
    }

    async fn run_with_evm<FEN: FoundryEvmNetwork>(self) -> Result<()> {
        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::from_provider(figment)?.sanitized();

        let label = self.label;
        let with_local_artifacts = self.with_local_artifacts;
        let debug = self.debug;
        let decode_internal = self.decode_internal;
        let disable_labels = self.disable_labels;
        let compute_units_per_second = if self.rpc.common.no_rpc_rate_limit {
            Some(u64::MAX)
        } else {
            self.rpc.common.compute_units_per_second
        };

        let provider = ProviderBuilder::<FEN::Network>::from_config(&config)?
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

        let tx_block_number = tx
            .block_number()
            .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;

        // we need to fork off the parent block
        config.fork_block_number = Some(tx_block_number - 1);

        let create2_deployer = evm_opts.create2_deployer;
        let (block, (mut evm_env, tx_env, fork, chain, networks)) = tokio::try_join!(
            // fetch the block the transaction was mined in
            provider.get_block(tx_block_number.into()).full().into_future().map_err(Into::into),
            TracingExecutor::<FEN>::get_fork_material(&mut config, evm_opts)
        )?;

        let mut evm_version = self.evm_version;

        evm_env.cfg_env.disable_block_gas_limit = self.disable_block_gas_limit;

        // By default do not enforce transaction gas limits imposed by Osaka (EIP-7825).
        // Users can opt-in to enable these limits by setting `enable_tx_gas_limit` to true.
        if !self.enable_tx_gas_limit {
            evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        }

        evm_env.cfg_env.limit_contract_code_size = None;
        evm_env.block_env.set_number(U256::from(tx_block_number));

        if let Some(block) = &block {
            evm_env.block_env = block_env_from_header(block.header());

            // Resolve the correct spec for the block using the same approach as reth: walk
            // known chain activation conditions to find the latest active fork. Falls back
            // to a blob-gas heuristic for unknown chains.
            if evm_version.is_none() {
                if let Some(hardfork) = FoundryHardfork::from_chain_and_timestamp(
                    evm_env.cfg_env.chain_id,
                    block.header().timestamp(),
                ) {
                    evm_env.cfg_env.set_spec_and_mainnet_gas_params(hardfork.into());
                } else if block.header().excess_blob_gas().is_some() {
                    // TODO: add glamsterdam header field checks in the future
                    evm_version = Some(EvmVersion::Cancun);
                }
            }
            apply_chain_and_block_specific_env_changes::<FEN::Network, _, _>(
                &mut evm_env,
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
        let mut executor = TracingExecutor::<FEN>::new(
            (evm_env.clone(), tx_env),
            fork,
            evm_version,
            trace_mode,
            networks,
            create2_deployer,
            None,
        )?;

        evm_env.cfg_env.set_spec_and_mainnet_gas_params(executor.spec_id());

        // Set the state to the moment right before the transaction
        if !self.quick {
            if !shell::is_json() {
                sh_println!("Executing previous transactions from the block.")?;
            }

            if let Some(block) = block {
                let pb = init_progress(block.transactions().len() as u64, "tx");
                pb.set_position(0);

                let BlockTransactions::Full(ref txs) = *block.transactions() else {
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

                    let tx_env = TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from());

                    evm_env.cfg_env.disable_balance_check = true;

                    if let Some(to) = Transaction::to(tx) {
                        trace!(tx=?tx.tx_hash(),?to, "executing previous call transaction");
                        executor.transact_with_env(evm_env.clone(), tx_env.clone()).wrap_err_with(
                            || {
                                format!(
                                    "Failed to execute transaction: {:?} in block {}",
                                    tx.tx_hash(),
                                    evm_env.block_env.number()
                                )
                            },
                        )?;
                    } else {
                        trace!(tx=?tx.tx_hash(), "executing previous create transaction");
                        if let Err(error) =
                            executor.deploy_with_env(evm_env.clone(), tx_env.clone(), None)
                        {
                            match error {
                                // Reverted transactions should be skipped
                                EvmError::Execution(_) => (),
                                error => {
                                    return Err(error).wrap_err_with(|| {
                                        format!(
                                            "Failed to deploy transaction: {:?} in block {}",
                                            tx.tx_hash(),
                                            evm_env.block_env.number()
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

            let tx_env = TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from());

            if tx.as_ref().recover_signer().is_ok_and(|signer| signer != tx.from()) {
                evm_env.cfg_env.disable_balance_check = true;
            }

            if let Some(to) = Transaction::to(&tx) {
                trace!(tx=?tx.tx_hash(), to=?to, "executing call transaction");
                TraceResult::from(executor.transact_with_env(evm_env, tx_env)?)
            } else {
                trace!(tx=?tx.tx_hash(), "executing create transaction");
                TraceResult::try_from(executor.deploy_with_env(evm_env, tx_env, None))?
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

pub fn fetch_contracts_bytecode_from_trace<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
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
