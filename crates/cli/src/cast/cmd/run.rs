use clap::Parser;
use ethers::{prelude::Middleware, solc::EvmVersion, types::H160};
use eyre::WrapErr;
use forge::{
    executor::{inspector::cheatcodes::util::configure_tx_env, opts::EvmOpts},
    revm::primitives::U256 as rU256,
    utils::h256_to_b256,
};
use foundry_cli::{
    init_progress,
    opts::RpcOpts,
    update_progress, utils,
    utils::{handle_traces, TraceResult},
};
use foundry_config::{find_project_root_path, Config};
use foundry_evm::{executor::EvmError, trace::TracingExecutor};
use tracing::trace;

const ARBITRUM_SENDER: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x0a, 0x4b, 0x05,
]);

/// CLI arguments for `cast run`.
#[derive(Debug, Clone, Parser)]
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
    /// See also, https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups
    #[clap(long, alias = "cups", value_name = "CUPS")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    ///
    /// default value: false
    ///
    /// See also, https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups
    #[clap(long, value_name = "NO_RATE_LIMITS", visible_alias = "no-rpc-rate-limit")]
    pub no_rate_limit: bool,
}

impl RunArgs {
    /// Executes the transaction by replaying it
    ///
    /// This replays the entire block the transaction was mined in unless `quick` is set to true
    ///
    /// Note: This executes the transaction(s) as is: Cheatcodes are disabled
    pub async fn run(self) -> eyre::Result<()> {
        let figment =
            Config::figment_with_root(find_project_root_path(None).unwrap()).merge(self.rpc);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = Config::from_provider(figment).sanitized();

        let compute_units_per_second =
            if self.no_rate_limit { Some(u64::MAX) } else { self.compute_units_per_second };

        let provider = utils::get_provider_builder(&config)?
            .compute_units_per_second_opt(compute_units_per_second)
            .build()?;

        let tx_hash = self.tx_hash.parse().wrap_err("invalid tx hash")?;
        let tx = provider
            .get_transaction(tx_hash)
            .await?
            .ok_or_else(|| eyre::eyre!("tx not found: {:?}", tx_hash))?;

        let tx_block_number = tx
            .block_number
            .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?
            .as_u64();

        config.fork_block_number = Some(tx_block_number - 1);

        let (mut env, fork, chain) = TracingExecutor::get_fork_material(&config, evm_opts).await?;

        let mut executor =
            TracingExecutor::new(env.clone(), fork, self.evm_version, self.debug).await;

        env.block.number = rU256::from(tx_block_number);

        let block = provider.get_block_with_txs(tx_block_number).await?;
        if let Some(ref block) = block {
            env.block.timestamp = block.timestamp.into();
            env.block.coinbase = block.author.unwrap_or_default().into();
            env.block.difficulty = block.difficulty.into();
            env.block.prevrandao = block.mix_hash.map(h256_to_b256);
            env.block.basefee = block.base_fee_per_gas.unwrap_or_default().into();
            env.block.gas_limit = block.gas_limit.into();
        }

        // Set the state to the moment right before the transaction
        if !self.quick {
            println!("Executing previous transactions from the block.");

            if let Some(block) = block {
                let pb = init_progress!(block.transactions, "tx");
                pb.set_position(0);

                for (index, tx) in block.transactions.into_iter().enumerate() {
                    // arbitrum L1 transaction at the start of every block that has gas price 0
                    // and gas limit 0 which causes reverts, so we skip it
                    if tx.from == ARBITRUM_SENDER {
                        update_progress!(pb, index);
                        continue
                    }
                    if tx.hash().eq(&tx_hash) {
                        break
                    }

                    configure_tx_env(&mut env, &tx);

                    if let Some(to) = tx.to {
                        trace!(tx=?tx.hash,?to, "executing previous call transaction");
                        executor.commit_tx_with_env(env.clone()).wrap_err_with(|| {
                            format!("Failed to execute transaction: {:?}", tx.hash())
                        })?;
                    } else {
                        trace!(tx=?tx.hash, "executing previous create transaction");
                        if let Err(error) = executor.deploy_with_env(env.clone(), None) {
                            match error {
                                // Reverted transactions should be skipped
                                EvmError::Execution(_) => (),
                                error => {
                                    return Err(error).wrap_err_with(|| {
                                        format!("Failed to deploy transaction: {:?}", tx.hash())
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
                trace!(tx=?tx.hash,to=?to, "executing call transaction");
                TraceResult::from(executor.commit_tx_with_env(env)?)
            } else {
                trace!(tx=?tx.hash, "executing create transaction");
                match executor.deploy_with_env(env, None) {
                    Ok(res) => TraceResult::from(res),
                    Err(err) => TraceResult::try_from(err)?,
                }
            }
        };

        handle_traces(result, &config, chain, self.label, self.verbose, self.debug).await?;

        Ok(())
    }
}
