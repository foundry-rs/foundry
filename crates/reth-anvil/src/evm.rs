use crate::{
    block_env::BlockEnvOverrides,
    config::NodeConfig,
    console::{ConsoleBuffer, ConsolePrinter, HARDHAT_CONSOLE_ADDRESS},
    fork::ForkInfo,
    impersonation::ImpersonationState,
    state::{SharedAnvilState, StateOverride},
};
use alloy_consensus::transaction::TxHashRef;
use alloy_eips::Decodable2718;
use alloy_evm::{
    Database, Evm, EvmEnv, EvmFactory, FromRecoveredTx, FromTxWithEncoded, InvalidTxError,
    RecoveredTx, TransactionEnvMut,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockExecutorFactory,
        BlockValidationError, ExecutableTx, GasOutput, StateDB,
    },
    precompiles::{DynPrecompile, PrecompilesMap},
};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types_engine::ExecutionData;
use eyre::Result;
use reth_ethereum::{
    evm::primitives::{
        ConfigureEngineEvm, ConfigureEvm, EvmEnvFor, ExecutableTxIterator, ExecutionCtxFor,
        NextBlockEnvAttributes, SenderRecoveryCache, execute::BlockAssembler,
    },
    node::{
        api::{BlockTy, NodePrimitives, PayloadTypes},
        builder::{BuilderContext, FullNodeTypes, NodeTypes, components::ExecutorBuilder},
    },
    primitives::{Recovered, SealedBlock, SealedHeader, SignedTransaction},
    storage::errors::any::AnyError,
};
use revm::{
    Database as RevmDatabase, Inspector,
    context::{
        Block as _, CfgEnv, DBErrorMarker,
        result::{InvalidTransaction, ResultAndState},
    },
    inspector::NoOpInspector,
    primitives::hardfork::SpecId,
    state::{Account, AccountInfo, Bytecode, EvmState, EvmStorageSlot, TransactionId},
};
use std::{
    collections::hash_map::Entry,
    fmt::{self, Debug},
    marker::PhantomData,
    sync::Arc,
};

/// Next-block attributes the block environment overrides can set.
pub trait AnvilNextBlockEnv: Clone {
    /// Sets the block timestamp.
    fn set_timestamp(&mut self, timestamp: u64);
    /// Sets the block beneficiary.
    fn set_suggested_fee_recipient(&mut self, recipient: Address);
    /// Sets the block prevrandao.
    fn set_prev_randao(&mut self, prev_randao: B256);
    /// Sets the block gas limit.
    fn set_gas_limit(&mut self, gas_limit: u64);
    /// Replaces the parent beacon block root, when the block has one.
    fn override_parent_beacon_block_root(&mut self, root: B256);
}

impl AnvilNextBlockEnv for NextBlockEnvAttributes {
    fn set_timestamp(&mut self, timestamp: u64) {
        self.timestamp = timestamp;
    }

    fn set_suggested_fee_recipient(&mut self, recipient: Address) {
        self.suggested_fee_recipient = recipient;
    }

    fn set_prev_randao(&mut self, prev_randao: B256) {
        self.prev_randao = prev_randao;
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
    }

    fn override_parent_beacon_block_root(&mut self, root: B256) {
        if self.parent_beacon_block_root.is_some() {
            self.parent_beacon_block_root = Some(root);
        }
    }
}

/// Execution payloads whose raw transactions the engine EVM config can decode.
pub trait AnvilExecutionPayload {
    /// Returns the encoded transactions of the payload.
    fn raw_transactions(&self) -> Vec<Bytes>;
}

impl AnvilExecutionPayload for ExecutionData {
    fn raw_transactions(&self) -> Vec<Bytes> {
        self.payload.transactions().clone()
    }
}

/// The EVM limits anvil relaxes or tightens: the contract code size limit, the memory limit, the
/// block gas limit check, and the per-transaction gas limit cap.
///
/// Applied to every EVM environment the node builds, so block execution and RPC calls agree.
#[derive(Clone, Copy, Debug, Default)]
pub struct EvmSettings {
    /// The contract code size limit. `None` keeps the hardfork's limit.
    pub code_size_limit: Option<usize>,
    /// The EVM memory limit in bytes. `None` keeps revm's default.
    pub memory_limit: Option<u64>,
    /// Whether a transaction may use more gas than the block gas limit.
    pub disable_block_gas_limit: bool,
    /// Whether the per-transaction gas limit cap of EIP-7825 is enforced.
    pub enable_tx_gas_limit: bool,
}

impl EvmSettings {
    /// Reads the settings from the node config.
    pub const fn from_config(config: &NodeConfig) -> Self {
        Self {
            code_size_limit: config.code_size_limit,
            memory_limit: config.memory_limit,
            disable_block_gas_limit: config.disable_block_gas_limit,
            enable_tx_gas_limit: config.enable_tx_gas_limit,
        }
    }

    /// Applies the settings to an EVM configuration environment.
    pub const fn apply<Spec>(&self, cfg: &mut CfgEnv<Spec>) {
        cfg.limit_contract_code_size = self.code_size_limit;
        // Accounts with code may send transactions, so impersonated contracts work.
        cfg.disable_eip3607 = true;
        cfg.disable_block_gas_limit = self.disable_block_gas_limit;
        if !self.enable_tx_gas_limit {
            cfg.tx_gas_limit_cap = Some(u64::MAX);
        }
        if let Some(memory_limit) = self.memory_limit {
            cfg.memory_limit = memory_limit;
        }
    }
}

/// Builds a precompile to install at an address, for the given block number.
pub type PrecompileBuilder = Arc<dyn Fn(u64) -> DynPrecompile + Send + Sync>;

/// EVM factory that installs extra precompiles, such as Celo's native transfer and the
/// `console.log` collector, into every EVM it creates, and that answers `BLOCKHASH` for the
/// blocks below a fork from the fork.
#[derive(Clone)]
pub struct AnvilEvmFactory<F> {
    inner: F,
    precompiles: Arc<Vec<(Address, PrecompileBuilder)>>,
    fork: Option<Arc<dyn ForkInfo>>,
    console: bool,
}

impl<F: Debug> Debug for AnvilEvmFactory<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let addresses: Vec<_> = self.precompiles.iter().map(|(address, _)| address).collect();
        f.debug_struct("AnvilEvmFactory")
            .field("inner", &self.inner)
            .field("precompiles", &addresses)
            .field("fork", &self.fork)
            .field("console", &self.console)
            .finish()
    }
}

impl<F> AnvilEvmFactory<F> {
    /// Wraps the factory with the precompiles to install, the fork, if any, and whether to
    /// collect `console.log` calls.
    pub fn new(
        inner: F,
        precompiles: Vec<(Address, PrecompileBuilder)>,
        fork: Option<Arc<dyn ForkInfo>>,
        console: bool,
    ) -> Self {
        Self { inner, precompiles: Arc::new(precompiles), fork, console }
    }

    /// Returns the wrapped factory.
    #[cfg(feature = "monad")]
    pub const fn inner(&self) -> &F {
        &self.inner
    }

    /// Installs the precompiles for the block and returns the `console.log` buffer, when
    /// collecting.
    fn install(
        &self,
        precompiles: &mut PrecompilesMap,
        block_number: u64,
    ) -> Option<ConsoleBuffer> {
        for (address, build) in self.precompiles.iter() {
            precompiles.apply_precompile(address, |_| Some(build(block_number)));
        }
        let console = self.console.then(ConsoleBuffer::default)?;
        precompiles.apply_precompile(&HARDHAT_CONSOLE_ADDRESS, |_| Some(console.precompile()));
        Some(console)
    }

    fn wrap_db<DB>(&self, db: DB) -> ForkHashDb<DB> {
        ForkHashDb { inner: db, fork: self.fork.clone() }
    }
}

impl<F> EvmFactory for AnvilEvmFactory<F>
where
    F: EvmFactory<Precompiles = PrecompilesMap>,
{
    type Evm<DB: Database, I: Inspector<F::Context<ForkHashDb<DB>>>> =
        AnvilEvm<F::Evm<ForkHashDb<DB>, I>, DB>;
    type Context<DB: Database> = F::Context<ForkHashDb<DB>>;
    type Tx = F::Tx;
    type Error<DBError: DBErrorMarker> = F::Error<DBError>;
    type HaltReason = F::HaltReason;
    type Spec = F::Spec;
    type BlockEnv = F::BlockEnv;
    type Precompiles = PrecompilesMap;

    fn create_evm<DB: Database>(
        &self,
        db: DB,
        input: EvmEnv<F::Spec, F::BlockEnv>,
    ) -> Self::Evm<DB, NoOpInspector> {
        let block_number = input.block_env.number().saturating_to();
        let mut evm = self.inner.create_evm(self.wrap_db(db), input);
        let console = self.install(evm.precompiles_mut(), block_number);
        AnvilEvm::new(evm, console)
    }

    fn create_evm_with_inspector<DB: Database, I: Inspector<F::Context<ForkHashDb<DB>>>>(
        &self,
        db: DB,
        input: EvmEnv<F::Spec, F::BlockEnv>,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        let block_number = input.block_env.number().saturating_to();
        let mut evm = self.inner.create_evm_with_inspector(self.wrap_db(db), input, inspector);
        let console = self.install(evm.precompiles_mut(), block_number);
        AnvilEvm::new(evm, console)
    }
}

/// Database adapter that serves the hashes of the blocks below the fork block from the fork.
///
/// The local database starts at the fork block, and the engine executes blocks against it
/// directly, so without the adapter `BLOCKHASH` of an older block reads as zero there while the
/// block builder, which reads through the fork, sees the real hash.
#[derive(Debug)]
pub struct ForkHashDb<DB> {
    inner: DB,
    fork: Option<Arc<dyn ForkInfo>>,
}

impl<DB: Database> RevmDatabase for ForkHashDb<DB> {
    type Error = DB::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.inner.basic(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.inner.code_by_hash(code_hash)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.inner.storage(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        if let Some(fork) = &self.fork
            && number < fork.block_number()
            && let Ok(Some(hash)) = fork.block_hash_by_number(number)
        {
            return Ok(hash);
        }
        self.inner.block_hash(number)
    }
}

/// EVM over a [`ForkHashDb`] that exposes the wrapped database as its own, with the
/// `console.log` lines of the transaction it executes.
pub struct AnvilEvm<E, DB> {
    inner: E,
    console: Option<ConsoleBuffer>,
    _db: PhantomData<fn() -> DB>,
}

impl<E: Debug, DB> Debug for AnvilEvm<E, DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilEvm")
            .field("inner", &self.inner)
            .field("console", &self.console)
            .finish()
    }
}

impl<E, DB> AnvilEvm<E, DB> {
    /// Wraps the EVM.
    pub const fn new(inner: E, console: Option<ConsoleBuffer>) -> Self {
        Self { inner, console, _db: PhantomData }
    }

    /// Returns the `console.log` buffer, when collecting.
    pub const fn console(&self) -> Option<&ConsoleBuffer> {
        self.console.as_ref()
    }

    /// Returns the wrapped EVM mutably.
    #[cfg(feature = "monad")]
    pub const fn inner_mut(&mut self) -> &mut E {
        &mut self.inner
    }
}

impl<E, DB> Evm for AnvilEvm<E, DB>
where
    DB: Database,
    E: Evm<DB = ForkHashDb<DB>>,
{
    type DB = DB;
    type Tx = E::Tx;
    type Error = E::Error;
    type HaltReason = E::HaltReason;
    type Spec = E::Spec;
    type BlockEnv = E::BlockEnv;
    type Precompiles = E::Precompiles;
    type Inspector = E::Inspector;

    fn block(&self) -> &Self::BlockEnv {
        self.inner.block()
    }

    fn cfg_env(&self) -> &CfgEnv<Self::Spec> {
        self.inner.cfg_env()
    }

    fn chain_id(&self) -> u64 {
        self.inner.chain_id()
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.transact_raw(tx)
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        self.inner.transact_system_call(caller, contract, data)
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec, Self::BlockEnv>) {
        let (db, env) = self.inner.finish();
        (db.inner, env)
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.inner.set_inspector_enabled(enabled)
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        let (db, inspector, precompiles) = self.inner.components();
        (&db.inner, inspector, precompiles)
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        let (db, inspector, precompiles) = self.inner.components_mut();
        (&mut db.inner, inspector, precompiles)
    }
}

/// Wraps an inner EVM config to override sender recovery for impersonated transactions during
/// engine payload execution, to apply the block environment overrides for the gas limit and the
/// base fee, and to apply the [`EvmSettings`].
#[derive(Debug, Clone)]
pub struct AnvilEvmConfig<Evm: ConfigureEvm> {
    inner: Evm,
    executor_factory: AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>,
    state: ImpersonationState,
    block_env: BlockEnvOverrides,
    settings: EvmSettings,
    /// The node's sender cache. Impersonated senders are recorded here so RPC lookups that
    /// recover senders through the cache report the impersonated account.
    sender_cache: Option<SenderRecoveryCache>,
}

impl<Evm: ConfigureEvm<NextBlockEnvCtx: AnvilNextBlockEnv>> AnvilEvmConfig<Evm> {
    /// Applies the persistent block environment overrides, and the pending parent beacon block
    /// root, so the pending block shows the root the next mined block gets.
    fn next_block_attributes(&self, mut attributes: Evm::NextBlockEnvCtx) -> Evm::NextBlockEnvCtx {
        if let Some(gas_limit) = self.block_env.gas_limit() {
            attributes.set_gas_limit(gas_limit);
        }
        if let Some(root) = self.block_env.next_parent_beacon_block_root() {
            attributes.override_parent_beacon_block_root(root);
        }
        attributes
    }
}

impl<Evm: ConfigureEvm<BlockExecutorFactory: Clone>> AnvilEvmConfig<Evm> {
    /// Wraps the given EVM config.
    pub fn new(
        inner: Evm,
        state: ImpersonationState,
        block_env: BlockEnvOverrides,
        anvil_state: SharedAnvilState,
        settings: EvmSettings,
        sender_cache: Option<SenderRecoveryCache>,
        console: Option<ConsolePrinter>,
    ) -> Self {
        let executor_factory = AnvilBlockExecutorFactory::new(
            inner.block_executor_factory().clone(),
            anvil_state,
            block_env.clone(),
            console,
        );
        Self { inner, executor_factory, state, block_env, settings, sender_cache }
    }
}

impl<Evm, Factory> ConfigureEvm for AnvilEvmConfig<Evm>
where
    Factory: EvmFactory<
            Precompiles = PrecompilesMap,
            Tx: TransactionEnvMut
                    + FromRecoveredTx<SignedTxOf<Evm>>
                    + FromTxWithEncoded<SignedTxOf<Evm>>,
            Spec: Into<SpecId>,
        >,
    Evm: ConfigureEvm<
            NextBlockEnvCtx: AnvilNextBlockEnv,
            BlockExecutorFactory: Clone
                                      + Debug
                                      + Send
                                      + Sync
                                      + Unpin
                                      + BlockExecutorFactory<
                Transaction: TxHashRef,
                EvmFactory = AnvilEvmFactory<Factory>,
            >,
            BlockAssembler: BlockAssembler<
                AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>,
                Block = <Evm::Primitives as NodePrimitives>::Block,
            >,
        >,
{
    type Primitives = Evm::Primitives;
    type Error = Evm::Error;
    type NextBlockEnvCtx = Evm::NextBlockEnvCtx;
    type BlockExecutorFactory = AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>;
    type BlockAssembler = Evm::BlockAssembler;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        &self.executor_factory
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(
        &self,
        header: &<Evm::Primitives as NodePrimitives>::BlockHeader,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        let mut env = self.inner.evm_env(header)?;
        self.settings.apply(&mut env.cfg_env);
        Ok(env)
    }

    fn next_evm_env(
        &self,
        parent: &<Evm::Primitives as NodePrimitives>::BlockHeader,
        attributes: &Self::NextBlockEnvCtx,
    ) -> Result<EvmEnvFor<Self>, Self::Error> {
        let attributes = self.next_block_attributes(attributes.clone());
        let mut env = self.inner.next_evm_env(parent, &attributes)?;
        // The block under construction gets its override; the pending block shows the next one.
        let base_fee = self.block_env.building_base_fee().or(self.block_env.next_base_fee());
        env.set_base_fee_opt(base_fee);
        self.settings.apply(&mut env.cfg_env);
        Ok(env)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<<Evm::Primitives as NodePrimitives>::Block>,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader<<Evm::Primitives as NodePrimitives>::BlockHeader>,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<ExecutionCtxFor<'_, Self>, Self::Error> {
        self.inner.context_for_next_block(parent, self.next_block_attributes(attributes))
    }
}

impl<Evm, Factory, Payload> ConfigureEngineEvm<Payload> for AnvilEvmConfig<Evm>
where
    Factory: EvmFactory<
            Precompiles = PrecompilesMap,
            Tx: TransactionEnvMut
                    + FromRecoveredTx<SignedTxOf<Evm>>
                    + FromTxWithEncoded<SignedTxOf<Evm>>,
            Spec: Into<SpecId>,
        >,
    Evm: ConfigureEvm<
            NextBlockEnvCtx: AnvilNextBlockEnv,
            BlockExecutorFactory: Clone
                                      + Debug
                                      + Send
                                      + Sync
                                      + Unpin
                                      + BlockExecutorFactory<
                Transaction: TxHashRef,
                EvmFactory = AnvilEvmFactory<Factory>,
            >,
            BlockAssembler: BlockAssembler<
                AnvilBlockExecutorFactory<Evm::BlockExecutorFactory>,
                Block = <Evm::Primitives as NodePrimitives>::Block,
            >,
        > + ConfigureEngineEvm<Payload>,
    Payload: AnvilExecutionPayload,
{
    fn evm_env_for_payload(&self, payload: &Payload) -> Result<EvmEnvFor<Self>, Self::Error> {
        let mut env = self.inner.evm_env_for_payload(payload)?;
        self.settings.apply(&mut env.cfg_env);
        Ok(env)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a Payload,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &Payload,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        let txs = payload.raw_transactions();
        let state = self.state.clone();
        let sender_cache = self.sender_cache.clone();

        let convert = move |raw: Bytes| {
            let tx = <Evm::Primitives as NodePrimitives>::SignedTx::decode_2718_exact(raw.as_ref())
                .map_err(AnyError::new)?;
            let signer = match state.tx_sender(&tx.recalculate_hash()) {
                Some(sender) => {
                    if let Some(cache) = &sender_cache {
                        let _ = cache.recover_with(&tx, |_| Ok(sender));
                    }
                    sender
                }
                None => match &sender_cache {
                    Some(cache) => cache.recover(&tx).map_err(AnyError::new)?,
                    None => tx.try_recover().map_err(AnyError::new)?,
                },
            };
            Ok::<_, AnyError>(Recovered::new_unchecked(tx, signer))
        };

        Ok((txs, convert))
    }
}

/// Block executor factory that applies the queued anvil state writes at the start of every
/// block, caps the number of transactions per block, and prints `console.log` output.
#[derive(Debug, Clone)]
pub struct AnvilBlockExecutorFactory<F> {
    inner: F,
    state: SharedAnvilState,
    block_env: BlockEnvOverrides,
    console: Option<ConsolePrinter>,
}

impl<F> AnvilBlockExecutorFactory<F> {
    /// Wraps the given factory.
    pub const fn new(
        inner: F,
        state: SharedAnvilState,
        block_env: BlockEnvOverrides,
        console: Option<ConsolePrinter>,
    ) -> Self {
        Self { inner, state, block_env, console }
    }
}

impl<F, Evm> BlockExecutorFactory for AnvilBlockExecutorFactory<F>
where
    F: BlockExecutorFactory<Transaction: TxHashRef, EvmFactory = AnvilEvmFactory<Evm>>,
    Evm: EvmFactory<
            Precompiles = PrecompilesMap,
            Tx: FromRecoveredTx<F::Transaction> + FromTxWithEncoded<F::Transaction>,
        >,
{
    type EvmFactory = F::EvmFactory;
    type TxExecutionResult = F::TxExecutionResult;
    type ExecutionCtx<'a> = F::ExecutionCtx<'a>;
    type Transaction = F::Transaction;
    type Receipt = F::Receipt;
    type Executor<'a, DB: StateDB, I: Inspector<<Self::EvmFactory as EvmFactory>::Context<DB>>> =
        AnvilBlockExecutor<F::Executor<'a, DB, I>>;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: <Self::EvmFactory as EvmFactory>::Evm<DB, I>,
        ctx: Self::ExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<<Self::EvmFactory as EvmFactory>::Context<DB>>,
    {
        AnvilBlockExecutor {
            inner: self.inner.create_executor(evm, ctx),
            state: self.state.clone(),
            max_transactions: self.block_env.max_transactions(),
            console: self.console.clone(),
            current_tx: None,
        }
    }
}

/// Block executor that applies the queued anvil state writes after the pre-execution changes,
/// rejects every transaction past the block's transaction count limit, and prints the
/// `console.log` lines of every committed transaction.
///
/// The payload builder skips a rejected transaction and leaves it in the pool for the next
/// block, as anvil's miner does with the transactions past `--max-transactions`.
#[derive(Debug)]
pub struct AnvilBlockExecutor<E> {
    inner: E,
    state: SharedAnvilState,
    max_transactions: Option<usize>,
    console: Option<ConsolePrinter>,
    /// The hash of the transaction executed last, until it is committed.
    current_tx: Option<B256>,
}

impl<E, Inner, DB> BlockExecutor for AnvilBlockExecutor<E>
where
    E: BlockExecutor<Transaction: TxHashRef, Evm = AnvilEvm<Inner, DB>>,
    Inner: Evm<
            DB = ForkHashDb<DB>,
            Tx: FromRecoveredTx<E::Transaction> + FromTxWithEncoded<E::Transaction>,
        >,
    DB: StateDB,
{
    type Transaction = E::Transaction;
    type Receipt = E::Receipt;
    type Evm = E::Evm;
    type Result = E::Result;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        self.inner.apply_pre_execution_changes()?;
        let number = self.inner.evm().block().number().saturating_to::<u64>();
        let writes = self.state.write().overrides_for_block(number);
        if writes.is_empty() {
            return Ok(());
        }
        apply_state_writes(self.inner.evm_mut().db_mut(), &writes)
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        let (tx_env, recovered) = tx.into_parts();
        let hash = *recovered.tx().tx_hash();
        if let Some(limit) = self.max_transactions
            && self.inner.receipts().len() >= limit
        {
            return Err(BlockValidationError::InvalidTx {
                hash,
                error: Box::new(BlockFull(limit)),
            }
            .into());
        }
        if let Some(console) = self.inner.evm().console() {
            console.clear();
        }
        self.current_tx = Some(hash);
        self.inner.execute_transaction_without_commit((tx_env, recovered))
    }

    fn commit_transaction(&mut self, output: Self::Result) -> GasOutput {
        let gas = self.inner.commit_transaction(output);
        if let Some(printer) = &self.console
            && let Some(hash) = self.current_tx.take()
            && let Some(console) = self.inner.evm().console()
        {
            printer.print(hash, console.take());
        }
        gas
    }

    fn finish(
        self,
    ) -> Result<(Self::Evm, BlockExecutionResult<Self::Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.inner.evm()
    }

    fn receipts(&self) -> &[Self::Receipt] {
        self.inner.receipts()
    }
}

/// The block already holds the configured number of transactions.
#[derive(Debug)]
struct BlockFull(usize);

impl fmt::Display for BlockFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block holds the maximum of {} transactions", self.0)
    }
}

impl std::error::Error for BlockFull {}

impl InvalidTxError for BlockFull {
    fn as_invalid_tx_err(&self) -> Option<&InvalidTransaction> {
        None
    }
}

/// Executor builder that wraps a network's executor builder in an [`AnvilEvmConfig`].
#[derive(Debug, Clone)]
pub struct AnvilExecutorBuilder<Inner> {
    /// The network's executor builder.
    pub inner: Inner,
    /// The shared impersonation state.
    pub state: ImpersonationState,
    /// The shared block environment overrides.
    pub block_env: BlockEnvOverrides,
    /// The shared anvil state writes.
    pub anvil_state: SharedAnvilState,
    /// The EVM limits.
    pub settings: EvmSettings,
    /// The `console.log` printer, when printing.
    pub console: Option<ConsolePrinter>,
}

/// The signed transaction type of an EVM config.
type SignedTxOf<Evm> = <<Evm as ConfigureEvm>::Primitives as NodePrimitives>::SignedTx;

/// The execution payload of a node.
type ExecutionDataOf<Node> =
    <<<Node as FullNodeTypes>::Types as NodeTypes>::Payload as PayloadTypes>::ExecutionData;

impl<Node, Inner, Factory> ExecutorBuilder<Node> for AnvilExecutorBuilder<Inner>
where
    Factory: EvmFactory<
            Precompiles = PrecompilesMap,
            Tx: TransactionEnvMut
                    + FromRecoveredTx<SignedTxOf<Inner::EVM>>
                    + FromTxWithEncoded<SignedTxOf<Inner::EVM>>,
            Spec: Into<SpecId>,
        >,
    Node: FullNodeTypes,
    Inner: ExecutorBuilder<Node>,
    Inner::EVM: ConfigureEvm<
            Primitives = <Node::Types as NodeTypes>::Primitives,
            NextBlockEnvCtx: AnvilNextBlockEnv,
            BlockExecutorFactory: Clone
                                      + Debug
                                      + Send
                                      + Sync
                                      + Unpin
                                      + BlockExecutorFactory<
                Transaction: TxHashRef,
                EvmFactory = AnvilEvmFactory<Factory>,
            >,
            BlockAssembler: BlockAssembler<
                AnvilBlockExecutorFactory<<Inner::EVM as ConfigureEvm>::BlockExecutorFactory>,
                Block = BlockTy<Node::Types>,
            >,
        > + ConfigureEngineEvm<ExecutionDataOf<Node>>,
    ExecutionDataOf<Node>: AnvilExecutionPayload,
{
    type EVM = AnvilEvmConfig<Inner::EVM>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> Result<Self::EVM> {
        Ok(AnvilEvmConfig::new(
            self.inner.build_evm(ctx).await?,
            self.state,
            self.block_env,
            self.anvil_state,
            self.settings,
            ctx.sender_recovery_cache().cloned(),
            self.console,
        ))
    }
}

/// Commits the given state writes to the block's state database as one touched-account change set,
/// so they become part of the block's state transition.
fn apply_state_writes<DB: StateDB>(
    db: &mut DB,
    writes: &[StateOverride],
) -> Result<(), BlockExecutionError> {
    let mut changes = EvmState::default();
    for write in writes {
        let address = match write {
            StateOverride::Balance(address, _)
            | StateOverride::Nonce(address, _)
            | StateOverride::Code(address, _)
            | StateOverride::Storage(address, _, _) => *address,
        };
        let account = match changes.entry(address) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let info =
                    db.basic(address).map_err(BlockExecutionError::other)?.unwrap_or_default();
                let mut account = Account::from(info);
                account.mark_touch();
                entry.insert(account)
            }
        };
        match write {
            StateOverride::Balance(_, balance) => account.info.balance = *balance,
            StateOverride::Nonce(_, nonce) => account.info.nonce = *nonce,
            StateOverride::Code(_, code) => {
                account.info.code_hash = code.hash_slow();
                account.info.code = Some(code.0.clone());
            }
            StateOverride::Storage(_, slot, value) => {
                let slot = U256::from_be_bytes(slot.0);
                let original = db.storage(address, slot).map_err(BlockExecutionError::other)?;
                account.storage.insert(
                    slot,
                    EvmStorageSlot::new_changed(original, *value, TransactionId::ZERO),
                );
            }
        }
    }
    db.commit(changes);
    Ok(())
}
