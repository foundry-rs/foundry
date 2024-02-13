use alloy_primitives::U256;
use alloy_providers::provider::TempProvider;
use alloy_rpc_types::BlockTransactions;
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    init_progress,
    opts::RpcOpts,
    update_progress,
    utils::{handle_traces, TraceResult},
};
use foundry_common::{is_known_system_sender, SYSTEM_TRANSACTION_TYPE};
use foundry_compilers::EvmVersion;
use foundry_config::{find_project_root_path, Config};
use foundry_evm::{
    executors::{EvmError, TracingExecutor},
    opts::EvmOpts,
    utils::configure_tx_env,
};

/// CLI arguments for `cast run`.
#[derive(Clone, Debug, Parser)]
pub struct RunArgs {
    /// The transaction hash.
    tx_hash: String,

    /// Opens the transaction in the debugger.
    #[clap(long, short)]
    debug: bool,

    /// Print out opcode traces.
    #[clap(long, short)]
    trace_printer: bool,

    /// Executes the transaction only with the state from the previous block.
    ///
    /// May result in different results than the live execution!
    #[clap(long, short)]
    quick: bool,

    /// Prints the full address of the contract.
    #[clap(long, short)]
    verbose: bool,

    /// Label addresses in the trace.
    ///
    /// Example: 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth
    #[clap(long, short)]
    label: Vec<String>,

    #[clap(flatten)]
    rpc: RpcOpts,

    /// The evm version to use.
    ///
    /// Overrides the version specified in the config.
    #[clap(long, short)]
    evm_version: Option<EvmVersion>,
    /// Sets the number of assumed available compute units per second for this provider
    ///
    /// default value: 330
    ///
    /// See also, https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second
    #[clap(long, alias = "cups", value_name = "CUPS")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    ///
    /// default value: false
    ///
    /// See also, https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second
    #[clap(long, value_name = "NO_RATE_LIMITS", visible_alias = "no-rpc-rate-limit")]
    pub no_rate_limit: bool,
}

impl RunArgs {
    /// Executes the transaction by replaying it
    ///
    /// This replays the entire block the transaction was mined in unless `quick` is set to true
    ///
    /// Note: This executes the transaction(s) as is: Cheatcodes are disabled
    pub async fn run(self) -> Result<()> {
        let figment =
            Config::figment_with_root(find_project_root_path(None).unwrap()).merge(self.rpc);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::try_from(figment)?.sanitized();

        let compute_units_per_second =
            if self.no_rate_limit { Some(u64::MAX) } else { self.compute_units_per_second };

        let provider = foundry_common::provider::alloy::ProviderBuilder::new(
            &config.get_rpc_url_or_localhost_http()?,
        )
        .compute_units_per_second_opt(compute_units_per_second)
        .build()?;

        let tx_hash = self.tx_hash.parse().wrap_err("invalid tx hash")?;
        let tx = provider
            .get_transaction_by_hash(tx_hash)
            .await
            .wrap_err_with(|| format!("tx not found: {:?}", tx_hash))?;

        // check if the tx is a system transaction
        if is_known_system_sender(tx.from) ||
            tx.transaction_type.map(|ty| ty.to::<u64>()) == Some(SYSTEM_TRANSACTION_TYPE)
        {
            return Err(eyre::eyre!(
                "{:?} is a system transaction.\nReplaying system transactions is currently not supported.",
                tx.hash
            ));
        }

        let tx_block_number = tx
            .block_number
            .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?
            .to::<u64>();

        // we need to fork off the parent block
        config.fork_block_number = Some(tx_block_number - 1);

        let (mut env, fork, chain) = TracingExecutor::get_fork_material(&config, evm_opts).await?;

        let mut executor =
            TracingExecutor::new(env.clone(), fork, self.evm_version, self.debug).await;

        env.block.number = U256::from(tx_block_number);

        let block = provider.get_block(tx_block_number.into(), true).await?;
        if let Some(ref block) = block {
            env.block.timestamp = block.header.timestamp;
            env.block.coinbase = block.header.miner;
            env.block.difficulty = block.header.difficulty;
            env.block.prevrandao = Some(block.header.mix_hash.unwrap_or_default());
            env.block.basefee = block.header.base_fee_per_gas.unwrap_or_default();
            env.block.gas_limit = block.header.gas_limit;
        }

        // Set the state to the moment right before the transaction
        if !self.quick {
            println!("Executing previous transactions from the block.");

            if let Some(block) = block {
                let pb = init_progress!(block.transactions, "tx");
                pb.set_position(0);

                let BlockTransactions::Full(txs) = block.transactions else {
                    return Err(eyre::eyre!("Could not get block txs"))
                };

                for (index, tx) in txs.into_iter().enumerate() {
                    // System transactions such as on L2s don't contain any pricing info so
                    // we skip them otherwise this would cause
                    // reverts
                    if is_known_system_sender(tx.from) ||
                        tx.transaction_type.map(|ty| ty.to::<u64>()) ==
                            Some(SYSTEM_TRANSACTION_TYPE)
                    {
                        update_progress!(pb, index);
                        continue;
                    }
                    if tx.hash == tx_hash {
                        break;
                    }

                    configure_tx_env(&mut env, &tx);

                    if let Some(to) = tx.to {
                        trace!(tx=?tx.hash,?to, "executing previous call transaction");
                        executor.commit_tx_with_env(env.clone()).wrap_err_with(|| {
                            format!(
                                "Failed to execute transaction: {:?} in block {}",
                                tx.hash, env.block.number
                            )
                        })?;
                    } else {
                        trace!(tx=?tx.hash, "executing previous create transaction");
                        if let Err(error) = executor.deploy_with_env(env.clone(), None) {
                            match error {
                                // Reverted transactions should be skipped
                                EvmError::Execution(_) => (),
                                error => {
                                    return Err(error).wrap_err_with(|| {
                                        format!(
                                            "Failed to deploy transaction: {:?} in block {}",
                                            tx.hash, env.block.number
                                        )
                                    })
                                }
                            }
                        }
                    }

                    update_progress!(pb, index);
                }
            }
        }

        // Execute our transaction
        let result = {
            executor.set_trace_printer(self.trace_printer);

            configure_tx_env(&mut env, &tx);

            if let Some(to) = tx.to {
                trace!(tx=?tx.hash, to=?to, "executing call transaction");
                TraceResult::from(executor.commit_tx_with_env(env)?)
            } else {
                trace!(tx=?tx.hash, "executing create transaction");
                match executor.deploy_with_env(env, None) {
                    Ok(res) => TraceResult::from(res),
                    Err(err) => TraceResult::try_from(err)?,
                }
            }
        };

        handle_traces(result, &config, chain, self.label, self.debug).await?;

        Ok(())
    }
}
