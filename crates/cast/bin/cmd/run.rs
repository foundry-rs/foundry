use alloy_consensus::Transaction;
use alloy_network::TransactionResponse;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::BlockTransactions;
use cast::revm::primitives::EnvWithHandlerCfg;
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{handle_traces, init_progress, TraceResult},
};
use foundry_common::{is_known_system_sender, shell, SYSTEM_TRANSACTION_TYPE};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map},
        Figment, Metadata, Profile,
    },
    Config,
};
use foundry_evm::{
    executors::{EvmError, TracingExecutor},
    opts::EvmOpts,
    traces::{InternalTraceMode, TraceMode},
    utils::configure_tx_env,
};

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

    /// Print out opcode traces.
    #[arg(long, short)]
    trace_printer: bool,

    /// Executes the transaction only with the state from the previous block.
    ///
    /// May result in different results than the live execution!
    #[arg(long)]
    quick: bool,

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
    /// See also, https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second
    #[arg(long, alias = "cups", value_name = "CUPS")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    ///
    /// default value: false
    ///
    /// See also, https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second
    #[arg(long, value_name = "NO_RATE_LIMITS", visible_alias = "no-rpc-rate-limit")]
    pub no_rate_limit: bool,

    /// Enables Odyssey features.
    #[arg(long, alias = "alphanet")]
    pub odyssey: bool,

    /// Use current project artifacts for trace decoding.
    #[arg(long, visible_alias = "la")]
    pub with_local_artifacts: bool,
}

impl RunArgs {
    /// Executes the transaction by replaying it
    ///
    /// This replays the entire block the transaction was mined in unless `quick` is set to true
    ///
    /// Note: This executes the transaction(s) as is: Cheatcodes are disabled
    pub async fn run(self) -> Result<()> {
        let figment = Into::<Figment>::into(&self.rpc).merge(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::try_from(figment)?.sanitized();

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
        if is_known_system_sender(tx.from) || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE)
        {
            return Err(eyre::eyre!(
                "{:?} is a system transaction.\nReplaying system transactions is currently not supported.",
                tx.tx_hash()
            ));
        }

        let tx_block_number =
            tx.block_number.ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;

        // fetch the block the transaction was mined in
        let block = provider.get_block(tx_block_number.into(), true.into()).await?;

        // we need to fork off the parent block
        config.fork_block_number = Some(tx_block_number - 1);

        let create2_deployer = evm_opts.create2_deployer;
        let (mut env, fork, chain, odyssey) =
            TracingExecutor::get_fork_material(&config, evm_opts).await?;

        let mut evm_version = self.evm_version;

        env.block.number = U256::from(tx_block_number);

        if let Some(block) = &block {
            env.block.timestamp = U256::from(block.header.timestamp);
            env.block.coinbase = block.header.beneficiary;
            env.block.difficulty = block.header.difficulty;
            env.block.prevrandao = Some(block.header.mix_hash.unwrap_or_default());
            env.block.basefee = U256::from(block.header.base_fee_per_gas.unwrap_or_default());
            env.block.gas_limit = U256::from(block.header.gas_limit);

            // TODO: we need a smarter way to map the block to the corresponding evm_version for
            // commonly used chains
            if evm_version.is_none() {
                // if the block has the excess_blob_gas field, we assume it's a Cancun block
                if block.header.excess_blob_gas.is_some() {
                    evm_version = Some(EvmVersion::Cancun);
                }
            }
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
            odyssey,
            create2_deployer,
        );
        let mut env =
            EnvWithHandlerCfg::new_with_spec_id(Box::new(env.clone()), executor.spec_id());

        // Set the state to the moment right before the transaction
        if !self.quick {
            if !shell::is_json() {
                sh_println!("Executing previous transactions from the block.")?;
            }

            if let Some(block) = block {
                let pb = init_progress(block.transactions.len() as u64, "tx");
                pb.set_position(0);

                let BlockTransactions::Full(ref txs) = block.transactions else {
                    return Err(eyre::eyre!("Could not get block txs"))
                };

                for (index, tx) in txs.iter().enumerate() {
                    // System transactions such as on L2s don't contain any pricing info so
                    // we skip them otherwise this would cause
                    // reverts
                    if is_known_system_sender(tx.from) ||
                        tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE)
                    {
                        pb.set_position((index + 1) as u64);
                        continue;
                    }
                    if tx.tx_hash() == tx_hash {
                        break;
                    }

                    configure_tx_env(&mut env, &tx.inner);

                    if let Some(to) = Transaction::to(tx) {
                        trace!(tx=?tx.tx_hash(),?to, "executing previous call transaction");
                        executor.transact_with_env(env.clone()).wrap_err_with(|| {
                            format!(
                                "Failed to execute transaction: {:?} in block {}",
                                tx.tx_hash(),
                                env.block.number
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
                                            env.block.number
                                        )
                                    })
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

            configure_tx_env(&mut env, &tx.inner);

            if let Some(to) = Transaction::to(&tx) {
                trace!(tx=?tx.tx_hash(), to=?to, "executing call transaction");
                TraceResult::try_from(executor.transact_with_env(env))?
            } else {
                trace!(tx=?tx.tx_hash(), "executing create transaction");
                TraceResult::try_from(executor.deploy_with_env(env, None))?
            }
        };

        handle_traces(
            result,
            &config,
            chain,
            self.label,
            self.with_local_artifacts,
            self.debug,
            self.decode_internal,
        )
        .await?;

        Ok(())
    }
}

impl figment::Provider for RunArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("RunArgs")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut map = Map::new();

        if self.odyssey {
            map.insert("odyssey".into(), self.odyssey.into());
        }

        if let Some(api_key) = &self.etherscan.key {
            map.insert("etherscan_api_key".into(), api_key.as_str().into());
        }

        if let Some(evm_version) = self.evm_version {
            map.insert("evm_version".into(), figment::value::Value::serialize(evm_version)?);
        }

        Ok(Map::from([(Config::selected_profile(), map)]))
    }
}
