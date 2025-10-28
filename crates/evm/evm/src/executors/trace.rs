use crate::{
    Env,
    executors::{Executor, ExecutorBuilder},
};
use alloy_primitives::{Address, U256, map::HashMap};
use alloy_rpc_types::state::StateOverride;
use eyre::Context;
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{Chain, Config, utils::evm_spec_id};
use foundry_evm_core::{backend::Backend, fork::CreateFork, opts::EvmOpts};
use foundry_evm_networks::NetworkConfigs;
use foundry_evm_traces::TraceMode;
use revm::{primitives::hardfork::SpecId, state::Bytecode};
use std::ops::{Deref, DerefMut};

/// A default executor with tracing enabled
pub struct TracingExecutor {
    executor: Executor,
}

impl TracingExecutor {
    pub fn new(
        env: Env,
        fork: CreateFork,
        version: Option<EvmVersion>,
        trace_mode: TraceMode,
        networks: NetworkConfigs,
        create2_deployer: Address,
        state_overrides: Option<StateOverride>,
    ) -> eyre::Result<Self> {
        let db = Backend::spawn(Some(fork))?;
        // configures a bare version of the evm executor: no cheatcode inspector is enabled,
        // tracing will be enabled only for the targeted transaction
        let mut executor = ExecutorBuilder::new()
            .inspectors(|stack| {
                stack.trace_mode(trace_mode).networks(networks).create2_deployer(create2_deployer)
            })
            .spec_id(evm_spec_id(version.unwrap_or_default()))
            .build(env, db);

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
    pub fn spec_id(&self) -> SpecId {
        self.executor.spec_id()
    }

    /// uses the fork block number from the config
    pub async fn get_fork_material(
        config: &mut Config,
        mut evm_opts: EvmOpts,
    ) -> eyre::Result<(Env, CreateFork, Chain, NetworkConfigs)> {
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        evm_opts.fork_block_number = config.fork_block_number;

        let env = evm_opts.evm_env().await?;

        let fork = evm_opts.get_fork(config, env.clone()).unwrap();
        let networks = evm_opts.networks.with_chain_id(env.evm_env.cfg_env.chain_id);
        config.labels.extend(networks.precompiles_label());

        let chain = env.tx.chain_id.unwrap().into();
        Ok((env, fork, chain, networks))
    }
}

impl Deref for TracingExecutor {
    type Target = Executor;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

impl DerefMut for TracingExecutor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.executor
    }
}
