use crate::executors::{Executor, ExecutorBuilder};
use alloy_primitives::{Address, ChainId, U256, map::HashMap};
use alloy_rpc_types::state::StateOverride;
use eyre::{Context, ContextCompat};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{Chain, Config, evm_spec_id};
use foundry_evm_core::{
    backend::Backend,
    evm::{BlockEnvFor, EvmEnvFor, FoundryEvmNetwork, SpecFor, TxEnvFor},
    fork::CreateFork,
    opts::{EvmOpts, ExecutionSpecContext, resolve_execution_spec},
};
#[cfg(feature = "base")]
use foundry_evm_hardforks::BaseSpecId;
use foundry_evm_hardforks::{FoundryHardfork, TempoHardfork};
use foundry_evm_networks::NetworkConfigs;
use foundry_evm_traces::TraceRequirements;
use revm::state::Bytecode;
use std::ops::{Deref, DerefMut};

/// A default executor with tracing enabled
pub struct TracingExecutor<FEN: FoundryEvmNetwork> {
    executor: Executor<FEN>,
}

impl<FEN: FoundryEvmNetwork> TracingExecutor<FEN> {
    /// Creates a tracing executor from tooling resolved by concrete network dispatch.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        builder: ExecutorBuilder<FEN>,
        env: (EvmEnvFor<FEN>, TxEnvFor<FEN>),
        fork: CreateFork,
        version: Option<EvmVersion>,
        trace_requirements: TraceRequirements,
        networks: NetworkConfigs,
        create2_deployer: Address,
        state_overrides: Option<StateOverride>,
    ) -> eyre::Result<Self> {
        let db = Backend::spawn(Some(fork))?;
        // configures a bare version of the evm executor: no cheatcode and log_collector inspector
        // is enabled, tracing will be enabled only for the targeted transaction
        let mut executor = builder
            .inspectors(|stack| {
                stack.trace_requirements(trace_requirements).create2_deployer(create2_deployer)
            })
            .spec_id_opt(version.map(evm_spec_id::<SpecFor<FEN>>))
            .build(env.0, env.1, db, networks);

        if let Some(state_overrides) = state_overrides {
            apply_state_overrides(&mut executor, state_overrides)?;
        }

        Ok(Self { executor })
    }

    /// Returns the spec id of the executor
    pub const fn spec_id(&self) -> SpecFor<FEN> {
        self.executor.spec_id()
    }

    /// Resolves and applies the execution spec for the effective block environment.
    pub fn resolve_spec(
        config: &Config,
        networks: NetworkConfigs,
        evm_env: &mut EvmEnvFor<FEN>,
        evm_version: Option<EvmVersion>,
    ) -> Option<FoundryHardfork> {
        Self::resolve_spec_for_chain(
            config,
            networks,
            evm_env.cfg_env.chain_id,
            None,
            evm_env,
            evm_version,
        )
    }

    /// Resolves and applies the execution spec using the source chain's hardfork schedule.
    pub fn resolve_spec_for_chain(
        config: &Config,
        networks: NetworkConfigs,
        source_chain_id: ChainId,
        endpoint_hardfork: Option<FoundryHardfork>,
        evm_env: &mut EvmEnvFor<FEN>,
        evm_version: Option<EvmVersion>,
    ) -> Option<FoundryHardfork> {
        let explicit_hardfork =
            evm_version.and_then(|version| network_hardfork_from_evm_version(networks, version));
        resolve_execution_spec(
            config,
            networks,
            evm_env,
            ExecutionSpecContext::historical(source_chain_id, endpoint_hardfork),
            evm_version.map(evm_spec_id::<SpecFor<FEN>>),
            explicit_hardfork,
        )
    }

    /// Extends trace labels with the precompiles active at the resolved execution hardfork.
    pub fn extend_precompile_labels(
        config: &mut Config,
        networks: NetworkConfigs,
        resolved_hardfork: Option<FoundryHardfork>,
    ) {
        config.labels.extend(networks.precompiles_label(resolved_hardfork));
    }

    /// uses the fork block number from the config
    pub async fn get_fork_material(
        config: &mut Config,
        mut evm_opts: EvmOpts,
    ) -> eyre::Result<(
        EvmEnvFor<FEN>,
        TxEnvFor<FEN>,
        CreateFork,
        Chain,
        NetworkConfigs,
        Option<FoundryHardfork>,
    )> {
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        evm_opts.fork_block_number = config.fork_block_number;
        evm_opts.infer_network_from_fork().await?;
        let networks = evm_opts.networks;
        let (evm_env, tx_env, resolved) =
            evm_opts.env_resolved::<SpecFor<FEN>, BlockEnvFor<FEN>, TxEnvFor<FEN>>().await?;
        let resolved = resolved.context("fork context is missing for tracing executor")?;
        let fork = evm_opts
            .get_fork_resolved(config, evm_env.cfg_env.chain_id, Some(&resolved))
            .context("fork URL is missing for tracing executor")?;
        let fork_context = resolved.context();

        let chain = fork_context.source_chain_id.into();
        Ok((evm_env, tx_env, fork, chain, networks, fork_context.hardfork))
    }
}

fn apply_state_overrides<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    state_overrides: StateOverride,
) -> eyre::Result<()> {
    for (address, overrides) in state_overrides {
        if let Some(balance) = overrides.balance {
            executor.set_balance(address, balance)?;
        }
        if let Some(nonce) = overrides.nonce {
            executor.set_account_nonce(address, nonce)?;
        }
        if let Some(code) = overrides.code {
            let bytecode =
                Bytecode::new_raw_checked(code).wrap_err("invalid bytecode in state override")?;
            executor.set_code(address, bytecode)?;
        }
        if let Some(state) = overrides.state {
            let state: HashMap<U256, U256> =
                state.into_iter().map(|(slot, value)| (slot.into(), value.into())).collect();
            executor.set_storage(address, state)?;
        }
        if let Some(state_diff) = overrides.state_diff {
            for (slot, value) in state_diff {
                executor.set_storage_slot(address, slot.into(), value.into())?;
            }
        }
    }
    Ok(())
}

fn network_hardfork_from_evm_version(
    networks: NetworkConfigs,
    evm_version: EvmVersion,
) -> Option<FoundryHardfork> {
    if networks.is_tempo() {
        return Some(FoundryHardfork::Tempo(evm_spec_id::<TempoHardfork>(evm_version)));
    }
    #[cfg(feature = "base")]
    if networks.is_base() {
        return Some(FoundryHardfork::Base(evm_spec_id::<BaseSpecId>(evm_version).upgrade()));
    }
    #[cfg(feature = "monad")]
    if networks.is_monad() {
        return Some(FoundryHardfork::Monad(evm_spec_id::<foundry_evm_hardforks::MonadHardfork>(
            evm_version,
        )));
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_rpc_types::state::AccountOverride;
    use foundry_evm_core::{FoundryTransaction, evm::EthEvmNetwork};
    use revm::context::Transaction;

    #[test]
    fn state_override_nonce_does_not_modify_transaction_nonce() {
        let sender = Address::repeat_byte(0x11);
        let mut tx_env = TxEnvFor::<EthEvmNetwork>::default();
        tx_env.set_caller(sender);
        tx_env.set_nonce(7);
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut evm_env = EvmEnvFor::<EthEvmNetwork>::default();
        evm_env.cfg_env.disable_nonce_check = true;
        let mut executor =
            ExecutorBuilder::default().build(evm_env, tx_env, backend, NetworkConfigs::default());
        executor.set_gas_limit(1_000_000);
        executor.set_account_nonce(sender, 7).unwrap();

        let overridden = Address::repeat_byte(0x42);
        let mut state_overrides = StateOverride::default();
        state_overrides.insert(sender, AccountOverride { nonce: Some(100), ..Default::default() });
        state_overrides
            .insert(overridden, AccountOverride { nonce: Some(200), ..Default::default() });

        apply_state_overrides(&mut executor, state_overrides).unwrap();

        assert_eq!(executor.get_nonce(sender).unwrap(), 100);
        assert_eq!(executor.get_nonce(overridden).unwrap(), 200);
        assert_eq!(executor.tx_env().caller(), sender);
        assert_eq!(executor.tx_env().nonce(), 7);

        let result =
            executor.transact_raw(sender, overridden, Default::default(), U256::ZERO).unwrap();
        assert_eq!(result.tx_env.nonce(), 7);
    }
}
