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
use foundry_evm_hardforks::FoundryHardfork;
use foundry_evm_networks::{
    NetworkConfigs,
    celo::transfer::{CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL},
    resolved_precompile_labels,
};
use foundry_evm_traces::{TraceContext, TraceRequirements};
use revm::state::Bytecode;
use std::ops::{Deref, DerefMut};

/// A default executor with tracing enabled
pub struct TracingExecutor<FEN: FoundryEvmNetwork> {
    executor: Executor<FEN>,
}

/// Fork state and network context used to construct a tracing executor.
pub struct TracingFork<FEN: FoundryEvmNetwork> {
    pub evm_env: EvmEnvFor<FEN>,
    pub tx_env: TxEnvFor<FEN>,
    fork: CreateFork,
    context: TraceContext,
}

impl<FEN: FoundryEvmNetwork> TracingFork<FEN> {
    pub const fn context(&self) -> TraceContext {
        self.context
    }

    /// Resolves the execution spec and carries it into the trace decoding context.
    pub fn resolve_spec(&mut self, config: &Config, evm_version: Option<EvmVersion>) {
        let hardfork = TracingExecutor::<FEN>::resolve_spec_for_chain(
            config,
            self.context.chain().id(),
            self.context.hardfork(),
            &mut self.evm_env,
            evm_version,
        );
        self.context = self.context.with_hardfork(hardfork);
    }

    /// Adds labels for precompiles active in this trace context.
    pub fn extend_precompile_labels(&self, config: &mut Config) {
        TracingExecutor::<FEN>::extend_precompile_labels(
            config,
            self.context.networks(),
            self.context.hardfork(),
        );
    }

    /// Builds a tracing executor from this resolved fork.
    pub fn into_executor(
        self,
        builder: ExecutorBuilder<FEN>,
        trace_requirements: TraceRequirements,
        create2_deployer: Address,
        state_overrides: Option<StateOverride>,
    ) -> eyre::Result<TracingExecutor<FEN>> {
        TracingExecutor::new(
            builder,
            (self.evm_env, self.tx_env),
            self.fork,
            None,
            trace_requirements,
            self.context.networks(),
            create2_deployer,
            state_overrides,
        )
    }

    fn into_parts(
        self,
    ) -> (EvmEnvFor<FEN>, TxEnvFor<FEN>, CreateFork, Chain, NetworkConfigs, Option<FoundryHardfork>)
    {
        (
            self.evm_env,
            self.tx_env,
            self.fork,
            self.context.chain(),
            self.context.networks(),
            self.context.hardfork(),
        )
    }
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

    /// Resolves and applies the execution spec using the source chain's hardfork schedule.
    pub fn resolve_spec_for_chain(
        config: &Config,
        source_chain_id: ChainId,
        endpoint_hardfork: Option<FoundryHardfork>,
        evm_env: &mut EvmEnvFor<FEN>,
        evm_version: Option<EvmVersion>,
    ) -> Option<FoundryHardfork> {
        resolve_execution_spec(
            config.evm_version,
            config.hardfork,
            evm_env,
            ExecutionSpecContext::historical(source_chain_id, endpoint_hardfork),
            evm_version.map(evm_spec_id::<SpecFor<FEN>>),
        )
    }

    /// Extends trace labels with the precompiles active at the resolved execution hardfork.
    pub fn extend_precompile_labels(
        config: &mut Config,
        networks: NetworkConfigs,
        resolved_hardfork: Option<FoundryHardfork>,
    ) {
        config.labels.extend(resolved_precompile_labels(resolved_hardfork));
        // Celo shares the Ethereum spec and remains a separate inspector configuration.
        if networks.is_celo() {
            config.labels.insert(CELO_TRANSFER_ADDRESS, CELO_TRANSFER_LABEL.to_string());
        }
    }

    /// Resolves the fork state and trace context using the fork block number from the config.
    pub async fn get_fork(
        config: &mut Config,
        mut evm_opts: EvmOpts,
    ) -> eyre::Result<TracingFork<FEN>> {
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
        Ok(TracingFork {
            evm_env,
            tx_env,
            fork,
            context: TraceContext::new(chain, networks, fork_context.hardfork),
        })
    }

    /// Returns the tracing fork as its legacy tuple representation.
    pub async fn get_fork_material(
        config: &mut Config,
        evm_opts: EvmOpts,
    ) -> eyre::Result<(
        EvmEnvFor<FEN>,
        TxEnvFor<FEN>,
        CreateFork,
        Chain,
        NetworkConfigs,
        Option<FoundryHardfork>,
    )> {
        Ok(Self::get_fork(config, evm_opts).await?.into_parts())
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

    fn assert_trace_spec_authority<FEN>(
        networks: NetworkConfigs,
        configured: FoundryHardfork,
        evm_version: EvmVersion,
        expected_spec: SpecFor<FEN>,
        expected_hardfork: Option<FoundryHardfork>,
    ) where
        FEN: FoundryEvmNetwork,
        SpecFor<FEN>: std::fmt::Debug + PartialEq,
    {
        let mut config = Config { networks, hardfork: Some(configured), ..Default::default() };
        let mut env = EvmEnvFor::<FEN>::default();
        env.cfg_env.chain_id = 999_999;
        let hardfork = TracingExecutor::<FEN>::resolve_spec_for_chain(
            &config,
            1,
            Some(configured),
            &mut env,
            Some(evm_version),
        );
        assert_eq!(env.cfg_env.spec, expected_spec);
        assert_eq!(hardfork, expected_hardfork);
        assert_eq!(env.cfg_env.chain_id, 999_999);

        TracingExecutor::<FEN>::extend_precompile_labels(&mut config, networks, hardfork);
        assert_eq!(config.labels, resolved_precompile_labels(expected_hardfork));
        let context = TraceContext::new(Chain::from_id(1), networks, hardfork);
        let decoder = foundry_evm_traces::CallTraceDecoderBuilder::new()
            .with_networks(context.networks())
            .with_hardfork(context.hardfork())
            .build();
        assert_eq!(decoder.hardfork(), expected_hardfork);
    }

    #[test]
    fn trace_spec_ethereum_override_preserves_absent_metadata() {
        assert_trace_spec_authority::<EthEvmNetwork>(
            NetworkConfigs::default(),
            "ethereum:shanghai".parse().unwrap(),
            EvmVersion::Cancun,
            revm::primitives::hardfork::SpecId::CANCUN,
            None,
        );
    }

    #[test]
    fn trace_spec_tempo_override_reports_executed_hardfork() {
        let spec = evm_spec_id::<foundry_evm_hardforks::TempoHardfork>(EvmVersion::Cancun);
        assert_trace_spec_authority::<foundry_evm_core::evm::TempoEvmNetwork>(
            NetworkConfigs::with_tempo(),
            "tempo:T3".parse().unwrap(),
            EvmVersion::Cancun,
            spec,
            Some(spec.into()),
        );
    }

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
