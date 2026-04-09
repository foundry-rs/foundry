use crate::executors::{Executor, ExecutorBuilder};
use alloy_primitives::{Address, U256, map::HashMap};
use alloy_rpc_types::state::StateOverride;
use eyre::Context;
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{Chain, Config, evm_spec_id};
use foundry_evm_core::{
    backend::Backend,
    evm::{BlockEnvFor, EvmEnvFor, FoundryEvmNetwork, SpecFor, TxEnvFor},
    fork::CreateFork,
    opts::EvmOpts,
};
use foundry_evm_networks::NetworkConfigs;
use foundry_evm_traces::TraceMode;
use revm::{context::Transaction, state::Bytecode};
use std::ops::{Deref, DerefMut};

/// A default executor with tracing enabled
pub struct TracingExecutor<FEN: FoundryEvmNetwork> {
    executor: Executor<FEN>,
}

impl<FEN: FoundryEvmNetwork> TracingExecutor<FEN> {
    pub fn new(
        env: (EvmEnvFor<FEN>, TxEnvFor<FEN>),
        fork: CreateFork,
        version: Option<EvmVersion>,
        trace_mode: TraceMode,
        networks: NetworkConfigs,
        create2_deployer: Address,
        state_overrides: Option<StateOverride>,
    ) -> eyre::Result<Self> {
        let db = Backend::spawn(Some(fork))?;
        // configures a bare version of the evm executor: no cheatcode and log_collector inspector
        // is enabled, tracing will be enabled only for the targeted transaction
        let mut executor = ExecutorBuilder::default()
            .inspectors(|stack| {
                stack.trace_mode(trace_mode).networks(networks).create2_deployer(create2_deployer)
            })
            .spec_id_opt(version.map(evm_spec_id::<SpecFor<FEN>>))
            .build(env.0, env.1, db);

        // Apply the state overrides.
        if let Some(state_overrides) = state_overrides {
            for (address, overrides) in state_overrides {
                if let Some(balance) = overrides.balance {
                    executor.set_balance(address, balance)?;
                }
                if let Some(nonce) = overrides.nonce {
                    executor.set_nonce(address, nonce)?;
                }
                if let Some(code) = overrides.code {
                    let bytecode = Bytecode::new_raw_checked(code)
                        .wrap_err("invalid bytecode in state override")?;
                    executor.set_code(address, bytecode)?;
                }
                if let Some(state) = overrides.state {
                    let state: HashMap<U256, U256> = state
                        .into_iter()
                        .map(|(slot, value)| (slot.into(), value.into()))
                        .collect();
                    executor.set_storage(address, state)?;
                }
                if let Some(state_diff) = overrides.state_diff {
                    for (slot, value) in state_diff {
                        executor.set_storage_slot(address, slot.into(), value.into())?;
                    }
                }
            }
        }

        Ok(Self { executor })
    }

    /// Returns the spec id of the executor
    pub fn spec_id(&self) -> SpecFor<FEN> {
        self.executor.spec_id()
    }

    /// uses the fork block number from the config
    pub async fn get_fork_material(
        config: &mut Config,
        mut evm_opts: EvmOpts,
    ) -> eyre::Result<(EvmEnvFor<FEN>, TxEnvFor<FEN>, CreateFork, Chain, NetworkConfigs)> {
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        evm_opts.fork_block_number = config.fork_block_number;

        let (evm_env, tx_env, fork_block) =
            evm_opts.env::<SpecFor<FEN>, BlockEnvFor<FEN>, TxEnvFor<FEN>>().await?;

        let fork = evm_opts.get_fork(config, evm_env.cfg_env.chain_id, fork_block).unwrap();
        let networks = evm_opts.networks.with_chain_id(evm_env.cfg_env.chain_id);
        config.labels.extend(networks.precompiles_label());

        let chain = tx_env.chain_id().unwrap().into();
        Ok((evm_env, tx_env, fork, chain, networks))
    }
}

impl<FEN: FoundryEvmNetwork> Deref for TracingExecutor<FEN> {
    type Target = Executor<FEN>;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

impl<FEN: FoundryEvmNetwork> DerefMut for TracingExecutor<FEN> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.executor
    }
}
