//! Execution rules for transaction-hash replays that differ from the configured hardfork.
//!
//! Overrides select a block number, so a later block can return to an older configured hardfork,
//! even when both blocks have the same timestamp. Native Ethereum execution and assembly remain
//! responsible for each block's system calls and header fields.

use super::evm::AnvilEthEvmFactory;
use crate::{
    block_env::BlockEnvOverrides, config::NodeConfig, evm::AnvilEvmFactory, fork::ForkInfo,
};
use alloy_consensus::Header;
use alloy_evm::{
    Evm, EvmFactory,
    block::{BlockExecutionError, BlockExecutorFactory, StateDB},
    eth::{EthBlockExecutionCtx, EthBlockExecutor, EthBlockExecutorFactory, spec::EthExecutorSpec},
};
use alloy_primitives::Address;
use alloy_rpc_types_engine::ExecutionData;
use reth_ethereum::{
    Block, EthPrimitives, Receipt, TransactionSigned,
    chainspec::{
        ChainSpec, EthChainSpec, EthereumHardfork, EthereumHardforks, ForkCondition, Hardforks,
    },
    evm::{
        EthEvmConfig, RethReceiptBuilder,
        primitives::{
            ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
            NextBlockEnvAttributes,
            execute::{BlockAssembler, BlockAssemblerInput},
        },
    },
    primitives::{SealedBlock, SealedHeader},
};
use revm::Inspector;
use std::{collections::BTreeMap, convert::Infallible, fmt::Debug, sync::Arc};

type Factory = AnvilEvmFactory<AnvilEthEvmFactory>;
type Native<C> = EthEvmConfig<C, Factory>;
type NativeFactory<C> = EthBlockExecutorFactory<RethReceiptBuilder, Arc<C>, Factory>;

/// Ethereum's block-specific rules, shared by execution, payload construction, and validation.
#[derive(Clone, Debug, Default)]
pub(crate) struct ExecutionOverrides(pub(crate) BTreeMap<u64, Arc<ChainSpec>>);

impl ExecutionOverrides {
    pub(crate) fn new(config: &NodeConfig, fork: Option<&dyn ForkInfo>) -> eyre::Result<Self> {
        let mut specs = BTreeMap::new();
        if let Some(fork) = fork {
            for (number, hardfork) in fork.replay_hardforks() {
                let config = config.clone().with_hardfork(Some(hardfork.into()));
                specs.insert(number, config.chain_spec()?);
            }
        }
        Ok(Self(specs))
    }
}

#[derive(Clone, Debug)]
struct Configs<C> {
    configured: Native<C>,
    replay: BTreeMap<u64, Native<ChainSpec>>,
    block_env: BlockEnvOverrides,
}

/// Ethereum execution with a native chain spec for each replayed block.
#[derive(Clone, Debug)]
pub struct ReplayEvmConfig<C> {
    configs: Arc<Configs<C>>,
    executor: ReplayExecutorFactory<C>,
    assembler: ReplayBlockAssembler<C>,
}

impl<C> ReplayEvmConfig<C> {
    pub(crate) fn new(
        configured: Native<C>,
        overrides: ExecutionOverrides,
        block_env: BlockEnvOverrides,
    ) -> Self {
        let factory = configured.executor_factory.evm_factory();
        let replay = overrides
            .0
            .into_iter()
            .map(|(number, spec)| {
                (number, EthEvmConfig::new_with_evm_factory(spec, factory.clone()))
            })
            .collect();
        let configs = Arc::new(Configs { configured, replay, block_env });
        Self {
            executor: ReplayExecutorFactory(configs.clone()),
            assembler: ReplayBlockAssembler(configs.clone()),
            configs,
        }
    }

    pub(crate) fn execution_overrides(&self) -> ExecutionOverrides {
        ExecutionOverrides(
            self.configs
                .replay
                .iter()
                .map(|(number, config)| (*number, config.chain_spec().clone()))
                .collect(),
        )
    }
}

impl<C> ConfigureEvm for ReplayEvmConfig<C>
where
    C: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
{
    type Primitives = EthPrimitives;
    type Error = Infallible;
    type NextBlockEnvCtx = NextBlockEnvAttributes;
    type BlockExecutorFactory = ReplayExecutorFactory<C>;
    type BlockAssembler = ReplayBlockAssembler<C>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        &self.executor
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        &self.assembler
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnvFor<Self>, Self::Error> {
        match self.configs.replay.get(&header.number) {
            Some(config) => config.evm_env(header),
            None => self.configs.configured.evm_env(header),
        }
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        match self.configs.replay.get(&(parent.number + 1)) {
            Some(config) => config.next_evm_env(parent, attributes),
            None => self.configs.configured.next_evm_env(parent, attributes),
        }
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<Block>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.configs.configured.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        mut attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        if let Some(config) = self.configs.replay.get(&(parent.number + 1)) {
            if config.chain_spec().is_cancun_active_at_timestamp(attributes.timestamp) {
                attributes.parent_beacon_block_root = Some(
                    self.configs
                        .block_env
                        .building_parent_beacon_block_root()
                        .or(self.configs.block_env.next_parent_beacon_block_root())
                        .or(attributes.parent_beacon_block_root)
                        .unwrap_or_default(),
                );
            }
            config.context_for_next_block(parent, attributes)
        } else {
            self.configs.configured.context_for_next_block(parent, attributes)
        }
    }
}

impl<C> ConfigureEngineEvm<ExecutionData> for ReplayEvmConfig<C>
where
    C: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
{
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> Result<EvmEnvFor<Self>, Self::Error> {
        match self.configs.replay.get(&payload.payload.block_number()) {
            Some(config) => config.evm_env_for_payload(payload),
            None => self.configs.configured.evm_env_for_payload(payload),
        }
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a ExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.configs.configured.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        self.configs.configured.tx_iterator_for_payload(payload)
    }
}

/// Selects the source or configured rules before constructing the native block executor.
#[derive(Clone, Debug)]
pub struct ReplayExecutorFactory<C>(Arc<Configs<C>>);

impl<C> BlockExecutorFactory for ReplayExecutorFactory<C>
where
    C: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
{
    type EvmFactory = Factory;
    type ExecutionCtx<'a> = EthBlockExecutionCtx<'a>;
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type TxExecutionResult = <NativeFactory<C> as BlockExecutorFactory>::TxExecutionResult;
    type Executor<'a, DB: StateDB, I: Inspector<<Factory as EvmFactory>::Context<DB>>> =
        EthBlockExecutor<
            'a,
            <Factory as EvmFactory>::Evm<DB, I>,
            ExecutionSpec<'a, C>,
            &'a RethReceiptBuilder,
        >;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.0.configured.executor_factory.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: <Factory as EvmFactory>::Evm<DB, I>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<<Factory as EvmFactory>::Context<DB>>,
    {
        let number = evm.block().number.saturating_to();
        let spec = match self.0.replay.get(&number) {
            Some(config) => ExecutionSpec::Replay(config.chain_spec()),
            None => ExecutionSpec::Configured(self.0.configured.chain_spec()),
        };
        EthBlockExecutor::new(evm, ctx, spec, self.0.configured.executor_factory.receipt_builder())
    }
}

/// The rule set borrowed by a native Ethereum block executor.
#[derive(Clone, Debug)]
pub enum ExecutionSpec<'a, C> {
    Configured(&'a Arc<C>),
    Replay(&'a Arc<ChainSpec>),
}

impl<C: EthereumHardforks> EthereumHardforks for ExecutionSpec<'_, C> {
    fn ethereum_fork_activation(&self, fork: EthereumHardfork) -> ForkCondition {
        match self {
            Self::Configured(spec) => spec.ethereum_fork_activation(fork),
            Self::Replay(spec) => spec.ethereum_fork_activation(fork),
        }
    }
}

impl<C: EthExecutorSpec> EthExecutorSpec for ExecutionSpec<'_, C> {
    fn deposit_contract_address(&self) -> Option<Address> {
        match self {
            Self::Configured(spec) => spec.deposit_contract_address(),
            Self::Replay(spec) => spec.deposit_contract_address(),
        }
    }
}

/// Assembles each block with the same native rules used to execute it.
#[derive(Clone, Debug)]
pub struct ReplayBlockAssembler<C>(Arc<Configs<C>>);

impl<C> BlockAssembler<ReplayExecutorFactory<C>> for ReplayBlockAssembler<C>
where
    C: EthExecutorSpec + EthChainSpec<Header = Header> + Hardforks + 'static,
{
    type Block = Block;

    fn assemble_block(
        &self,
        input: BlockAssemblerInput<'_, '_, ReplayExecutorFactory<C>>,
    ) -> Result<Self::Block, BlockExecutionError> {
        let number = input.evm_env.block_env.number.saturating_to();
        match self.0.replay.get(&number) {
            Some(config) => config.block_assembler.assemble_block(input, None, None, None),
            None => self.0.configured.block_assembler.assemble_block(input, None, None, None),
        }
    }
}
