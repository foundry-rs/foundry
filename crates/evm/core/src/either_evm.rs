use alloy_evm::{Database, EthEvm, Evm, EvmEnv, eth::EthEvmContext};
use alloy_op_evm::OpEvm;
use alloy_primitives::{Address, Bytes};
use op_revm::{OpContext, OpHaltReason, OpSpecId, OpTransaction, OpTransactionError};
use revm::{
    DatabaseCommit, Inspector,
    context::{
        BlockEnv, TxEnv,
        result::{EVMError, ExecResultAndState, ExecutionResult, ResultAndState},
    },
    handler::PrecompileProvider,
    interpreter::InterpreterResult,
    primitives::hardfork::SpecId,
};

#[cfg(feature = "tempo")]
use revm::context::result::{HaltReason, InvalidTransaction};

#[cfg(feature = "tempo")]
use tempo_chainspec::hardfork::TempoHardfork;
#[cfg(feature = "tempo")]
use tempo_revm::{
    TempoBlockEnv, TempoEvm as InnerTempoEvm, TempoHaltReason, TempoInvalidTransaction, TempoTxEnv,
    evm::TempoContext,
};

#[cfg(feature = "tempo")]
use alloy_evm::precompiles::PrecompilesMap;
#[cfg(feature = "tempo")]
use revm::{Context, InspectSystemCallEvm, MainContext, inspector::NoOpInspector};

/// Alias for result type returned by [`Evm::transact`] methods.
type EitherEvmResult<DBError, HaltReason, TxError> =
    Result<ResultAndState<HaltReason>, EVMError<DBError, TxError>>;

/// Alias for result type returned by [`Evm::transact_commit`] methods.
type EitherExecResult<DBError, HaltReason, TxError> =
    Result<ExecutionResult<HaltReason>, EVMError<DBError, TxError>>;

/// Tempo EVM wrapper for use in foundry.
///
/// This wraps `tempo_revm::TempoEvm` and implements the `alloy_evm::Evm` trait,
/// similar to how `tempo-evm` crate does it.
#[cfg(feature = "tempo")]
pub struct FoundryTempoEvm<DB: Database, I = NoOpInspector> {
    inner: InnerTempoEvm<DB, I>,
    inspect: bool,
}

#[cfg(feature = "tempo")]
impl<DB: Database> FoundryTempoEvm<DB> {
    /// Create a new [`FoundryTempoEvm`] instance.
    pub fn new(db: DB, input: EvmEnv<TempoHardfork, TempoBlockEnv>) -> Self {
        let ctx = Context::mainnet()
            .with_db(db)
            .with_block(input.block_env)
            .with_cfg(input.cfg_env)
            .with_tx(Default::default());

        Self { inner: InnerTempoEvm::new(ctx, NoOpInspector {}), inspect: false }
    }
}

#[cfg(feature = "tempo")]
impl<DB: Database, I> FoundryTempoEvm<DB, I> {
    /// Provides a reference to the EVM context.
    pub const fn ctx(&self) -> &TempoContext<DB> {
        &self.inner.inner.ctx
    }

    /// Provides a mutable reference to the EVM context.
    pub fn ctx_mut(&mut self) -> &mut TempoContext<DB> {
        &mut self.inner.inner.ctx
    }

    /// Sets the inspector for the EVM.
    pub fn with_inspector<OINSP>(self, inspector: OINSP) -> FoundryTempoEvm<DB, OINSP> {
        FoundryTempoEvm { inner: self.inner.with_inspector(inspector), inspect: true }
    }
}

#[cfg(feature = "tempo")]
impl<DB, I> Evm for FoundryTempoEvm<DB, I>
where
    DB: Database,
    I: Inspector<TempoContext<DB>>,
{
    type DB = DB;
    type Tx = TempoTxEnv;
    type Error = EVMError<DB::Error, TempoInvalidTransaction>;
    type HaltReason = TempoHaltReason;
    type Spec = TempoHardfork;
    type BlockEnv = TempoBlockEnv;
    type Precompiles = PrecompilesMap;
    type Inspector = I;

    fn block(&self) -> &Self::BlockEnv {
        &self.ctx().block
    }

    fn chain_id(&self) -> u64 {
        self.ctx().cfg.chain_id
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        use alloy_primitives::TxKind;
        use revm::{ExecuteEvm, InspectEvm, SystemCallEvm};

        if tx.is_system_tx {
            let TxKind::Call(to) = tx.inner.kind else {
                return Err(TempoInvalidTransaction::SystemTransactionMustBeCall.into());
            };

            let mut result = if self.inspect {
                self.inner.inspect_system_call_with_caller(tx.inner.caller, to, tx.inner.data)?
            } else {
                self.inner.system_call_with_caller(tx.inner.caller, to, tx.inner.data)?
            };

            // system transactions should not consume any gas
            if let ExecutionResult::Success { gas_used, gas_refunded, .. } = &mut result.result {
                *gas_used = 0;
                *gas_refunded = 0;
            } else {
                return Err(TempoInvalidTransaction::SystemTransactionFailed(result.result).into());
            }

            Ok(result)
        } else if self.inspect {
            self.inner.inspect_tx(tx)
        } else {
            self.inner.transact(tx)
        }
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        use revm::SystemCallEvm;
        self.inner.system_call_with_caller(caller, contract, data)
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec, Self::BlockEnv>) {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.inner.ctx;
        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.inspect = enabled;
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        (
            &self.inner.inner.ctx.journaled_state.database,
            &self.inner.inner.inspector,
            &self.inner.inner.precompiles,
        )
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        (
            &mut self.inner.inner.ctx.journaled_state.database,
            &mut self.inner.inner.inspector,
            &mut self.inner.inner.precompiles,
        )
    }
}

/// [`EitherEvm`] delegates its calls to one of the two evm implementations; either [`EthEvm`] or
/// [`OpEvm`].
///
/// Calls are delegated to [`OpEvm`] only if optimism is enabled.
///
/// The call delegation is handled via its own implementation of the [`Evm`] trait.
///
/// The [`Evm::transact`] and other such calls work over the [`OpTransaction<TxEnv>`] type.
///
/// However, the [`Evm::HaltReason`] and [`Evm::Error`] leverage the optimism [`OpHaltReason`] and
/// [`OpTransactionError`] as these are supersets of the eth types. This makes it easier to map eth
/// types to op types and also prevents ignoring of any error that maybe thrown by [`OpEvm`].
#[allow(clippy::large_enum_variant)]
pub enum EitherEvm<DB, I, P>
where
    DB: Database,
{
    /// [`EthEvm`] implementation.
    Eth(EthEvm<DB, I, P>),
    /// [`OpEvm`] implementation.
    Op(OpEvm<DB, I, P>),
    /// [`FoundryTempoEvm`] implementation (Tempo chain support).
    ///
    /// Note: The `P` type parameter is unused for this variant since Tempo uses
    /// a fixed `PrecompilesMap` internally.
    #[cfg(feature = "tempo")]
    Tempo(FoundryTempoEvm<DB, I>, std::marker::PhantomData<P>),
}

#[cfg(not(feature = "tempo"))]
impl<DB, I, P> EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>,
{
    /// Converts the [`EthEvm::transact`] result to [`EitherEvmResult`].
    fn map_eth_result(
        &self,
        result: Result<ExecResultAndState<ExecutionResult>, EVMError<DB::Error>>,
    ) -> EitherEvmResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => Ok(ResultAndState {
                result: result.result.map_haltreason(OpHaltReason::Base),
                state: result.state,
            }),
            Err(e) => Err(self.map_eth_err(e)),
        }
    }

    /// Converts the [`EthEvm::transact_commit`] result to [`EitherExecResult`].
    fn map_exec_result(
        &self,
        result: Result<ExecutionResult, EVMError<DB::Error>>,
    ) -> EitherExecResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => {
                // Map the halt reason
                Ok(result.map_haltreason(OpHaltReason::Base))
            }
            Err(e) => Err(self.map_eth_err(e)),
        }
    }

    /// Maps [`EVMError<DBError>`] to [`EVMError<DBError, OpTransactionError>`].
    fn map_eth_err(&self, err: EVMError<DB::Error>) -> EVMError<DB::Error, OpTransactionError> {
        match err {
            EVMError::Transaction(invalid_tx) => {
                EVMError::Transaction(OpTransactionError::Base(invalid_tx))
            }
            EVMError::Database(e) => EVMError::Database(e),
            EVMError::Header(e) => EVMError::Header(e),
            EVMError::Custom(e) => EVMError::Custom(e),
        }
    }
}

#[cfg(feature = "tempo")]
impl<DB, I, P> EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>> + Inspector<TempoContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>,
{
    /// Converts the [`EthEvm::transact`] result to [`EitherEvmResult`].
    fn map_eth_result(
        &self,
        result: Result<ExecResultAndState<ExecutionResult>, EVMError<DB::Error>>,
    ) -> EitherEvmResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => Ok(ResultAndState {
                result: result.result.map_haltreason(OpHaltReason::Base),
                state: result.state,
            }),
            Err(e) => Err(self.map_eth_err(e)),
        }
    }

    /// Converts the [`EthEvm::transact_commit`] result to [`EitherExecResult`].
    fn map_exec_result(
        &self,
        result: Result<ExecutionResult, EVMError<DB::Error>>,
    ) -> EitherExecResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => {
                // Map the halt reason
                Ok(result.map_haltreason(OpHaltReason::Base))
            }
            Err(e) => Err(self.map_eth_err(e)),
        }
    }

    /// Maps [`EVMError<DBError>`] to [`EVMError<DBError, OpTransactionError>`].
    fn map_eth_err(&self, err: EVMError<DB::Error>) -> EVMError<DB::Error, OpTransactionError> {
        match err {
            EVMError::Transaction(invalid_tx) => {
                EVMError::Transaction(OpTransactionError::Base(invalid_tx))
            }
            EVMError::Database(e) => EVMError::Database(e),
            EVMError::Header(e) => EVMError::Header(e),
            EVMError::Custom(e) => EVMError::Custom(e),
        }
    }

    /// Converts the [`FoundryTempoEvm::transact`] result to [`EitherEvmResult`].
    #[cfg(feature = "tempo")]
    fn map_tempo_result(
        &self,
        result: Result<
            ResultAndState<TempoHaltReason>,
            EVMError<DB::Error, TempoInvalidTransaction>,
        >,
    ) -> EitherEvmResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => Ok(ResultAndState {
                result: result.result.map_haltreason(map_tempo_halt_reason),
                state: result.state,
            }),
            Err(e) => Err(self.map_tempo_err(e)),
        }
    }

    /// Converts the [`FoundryTempoEvm::transact_commit`] result to [`EitherExecResult`].
    #[cfg(feature = "tempo")]
    #[allow(dead_code)]
    fn map_tempo_exec_result(
        &self,
        result: Result<
            ExecutionResult<TempoHaltReason>,
            EVMError<DB::Error, TempoInvalidTransaction>,
        >,
    ) -> EitherExecResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => Ok(result.map_haltreason(map_tempo_halt_reason)),
            Err(e) => Err(self.map_tempo_err(e)),
        }
    }

    /// Maps [`EVMError<DBError, TempoInvalidTransaction>`] to [`EVMError<DBError,
    /// OpTransactionError>`].
    #[cfg(feature = "tempo")]
    fn map_tempo_err(
        &self,
        err: EVMError<DB::Error, TempoInvalidTransaction>,
    ) -> EVMError<DB::Error, OpTransactionError> {
        match err {
            EVMError::Transaction(tempo_tx_err) => {
                EVMError::Transaction(map_tempo_tx_error(tempo_tx_err))
            }
            EVMError::Database(e) => EVMError::Database(e),
            EVMError::Header(e) => EVMError::Header(e),
            EVMError::Custom(e) => EVMError::Custom(e),
        }
    }
}

/// Maps [`TempoHaltReason`] to [`OpHaltReason`].
#[cfg(feature = "tempo")]
fn map_tempo_halt_reason(halt: TempoHaltReason) -> OpHaltReason {
    match halt {
        TempoHaltReason::Ethereum(h) => OpHaltReason::Base(h),
        TempoHaltReason::SubblockTxFeePayment => {
            // Map Tempo-specific halt reason to a generic halt
            // SubblockTxFeePayment indicates a subblock transaction failed to pay fees
            OpHaltReason::Base(HaltReason::OutOfFunds)
        }
    }
}

/// Maps [`TempoInvalidTransaction`] to [`OpTransactionError`].
#[cfg(feature = "tempo")]
fn map_tempo_tx_error(err: TempoInvalidTransaction) -> OpTransactionError {
    match err {
        TempoInvalidTransaction::EthInvalidTransaction(e) => OpTransactionError::Base(e),
        // Map all other Tempo-specific errors to OpTransactionError::Base with a custom error
        // We use LackOfFundForMaxFee as a fallback for fee-related errors
        TempoInvalidTransaction::CollectFeePreTx(_) => {
            OpTransactionError::Base(InvalidTransaction::LackOfFundForMaxFee {
                fee: Box::new(alloy_primitives::U256::ZERO),
                balance: Box::new(alloy_primitives::U256::ZERO),
            })
        }
        // For other Tempo-specific errors, we map to a generic rejection
        _ => OpTransactionError::Base(InvalidTransaction::RejectCallerWithCode),
    }
}

/// Evm implementation for EitherEvm without Tempo support.
#[cfg(not(feature = "tempo"))]
impl<DB, I, P> Evm for EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>,
{
    type DB = DB;
    type Error = EVMError<DB::Error, OpTransactionError>;
    type HaltReason = OpHaltReason;
    type Tx = OpTransaction<TxEnv>;
    type Inspector = I;
    type Precompiles = P;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;

    fn block(&self) -> &BlockEnv {
        match self {
            Self::Eth(evm) => evm.block(),
            Self::Op(evm) => evm.block(),
        }
    }

    fn chain_id(&self) -> u64 {
        match self {
            Self::Eth(evm) => evm.chain_id(),
            Self::Op(evm) => evm.chain_id(),
        }
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        match self {
            Self::Eth(evm) => evm.components(),
            Self::Op(evm) => evm.components(),
        }
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        match self {
            Self::Eth(evm) => evm.components_mut(),
            Self::Op(evm) => evm.components_mut(),
        }
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        match self {
            Self::Eth(evm) => evm.db_mut(),
            Self::Op(evm) => evm.db_mut(),
        }
    }

    fn into_db(self) -> Self::DB
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.into_db(),
            Self::Op(evm) => evm.into_db(),
        }
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.finish(),
            Self::Op(evm) => {
                let (db, env) = evm.finish();
                (db, map_env(env))
            }
        }
    }

    fn precompiles(&self) -> &Self::Precompiles {
        match self {
            Self::Eth(evm) => evm.precompiles(),
            Self::Op(evm) => evm.precompiles(),
        }
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        match self {
            Self::Eth(evm) => evm.precompiles_mut(),
            Self::Op(evm) => evm.precompiles_mut(),
        }
    }

    fn inspector(&self) -> &Self::Inspector {
        match self {
            Self::Eth(evm) => evm.inspector(),
            Self::Op(evm) => evm.inspector(),
        }
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        match self {
            Self::Eth(evm) => evm.inspector_mut(),
            Self::Op(evm) => evm.inspector_mut(),
        }
    }

    fn enable_inspector(&mut self) {
        match self {
            Self::Eth(evm) => evm.enable_inspector(),
            Self::Op(evm) => evm.enable_inspector(),
        }
    }

    fn disable_inspector(&mut self) {
        match self {
            Self::Eth(evm) => evm.disable_inspector(),
            Self::Op(evm) => evm.disable_inspector(),
        }
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        match self {
            Self::Eth(evm) => evm.set_inspector_enabled(enabled),
            Self::Op(evm) => evm.set_inspector_enabled(enabled),
        }
    }

    fn into_env(self) -> EvmEnv<Self::Spec>
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.into_env(),
            Self::Op(evm) => map_env(evm.into_env()),
        }
    }

    fn transact(
        &mut self,
        tx: impl alloy_evm::IntoTxEnv<Self::Tx>,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            Self::Eth(evm) => {
                let eth = evm.transact(tx.into_tx_env().base);
                self.map_eth_result(eth)
            }
            Self::Op(evm) => evm.transact(tx),
        }
    }

    fn transact_commit(
        &mut self,
        tx: impl alloy_evm::IntoTxEnv<Self::Tx>,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error>
    where
        Self::DB: DatabaseCommit,
    {
        match self {
            Self::Eth(evm) => {
                let eth = evm.transact_commit(tx.into_tx_env().base);
                self.map_exec_result(eth)
            }
            Self::Op(evm) => evm.transact_commit(tx),
        }
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            Self::Eth(evm) => {
                let res = evm.transact_raw(tx.base);
                self.map_eth_result(res)
            }
            Self::Op(evm) => evm.transact_raw(tx),
        }
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            Self::Eth(evm) => {
                let eth = evm.transact_system_call(caller, contract, data);
                self.map_eth_result(eth)
            }
            Self::Op(evm) => evm.transact_system_call(caller, contract, data),
        }
    }
}

/// Evm implementation for EitherEvm with Tempo support.
#[cfg(feature = "tempo")]
impl<DB, I, P> Evm for EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>> + Inspector<TempoContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>,
{
    type DB = DB;
    type Error = EVMError<DB::Error, OpTransactionError>;
    type HaltReason = OpHaltReason;
    type Tx = OpTransaction<TxEnv>;
    type Inspector = I;
    type Precompiles = P;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;

    fn block(&self) -> &BlockEnv {
        match self {
            Self::Eth(evm) => evm.block(),
            Self::Op(evm) => evm.block(),
            Self::Tempo(evm, _) => &evm.ctx().block.inner,
        }
    }

    fn chain_id(&self) -> u64 {
        match self {
            Self::Eth(evm) => evm.chain_id(),
            Self::Op(evm) => evm.chain_id(),
            Self::Tempo(evm, _) => evm.ctx().cfg.chain_id,
        }
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        match self {
            Self::Eth(evm) => evm.components(),
            Self::Op(evm) => evm.components(),
            Self::Tempo(_, _) => {
                // Tempo uses PrecompilesMap which may not be compatible with P
                // This is a limitation of the current design
                panic!(
                    "components() not supported for Tempo variant - use db_mut() and inspector_mut() instead"
                )
            }
        }
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        match self {
            Self::Eth(evm) => evm.components_mut(),
            Self::Op(evm) => evm.components_mut(),
            Self::Tempo(_, _) => {
                // Tempo uses PrecompilesMap which may not be compatible with P
                panic!(
                    "components_mut() not supported for Tempo variant - use db_mut() and inspector_mut() instead"
                )
            }
        }
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        match self {
            Self::Eth(evm) => evm.db_mut(),
            Self::Op(evm) => evm.db_mut(),
            Self::Tempo(evm, _) => &mut evm.inner.inner.ctx.journaled_state.database,
        }
    }

    fn into_db(self) -> Self::DB
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.into_db(),
            Self::Op(evm) => evm.into_db(),
            Self::Tempo(evm, _) => evm.inner.inner.ctx.journaled_state.database,
        }
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.finish(),
            Self::Op(evm) => {
                let (db, env) = evm.finish();
                (db, map_env(env))
            }
            Self::Tempo(evm, _) => {
                let (db, env) = Evm::finish(evm);
                (db, map_tempo_env(env))
            }
        }
    }

    fn precompiles(&self) -> &Self::Precompiles {
        match self {
            Self::Eth(evm) => evm.precompiles(),
            Self::Op(evm) => evm.precompiles(),
            Self::Tempo(_, _) => {
                panic!("precompiles() not supported for Tempo variant")
            }
        }
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        match self {
            Self::Eth(evm) => evm.precompiles_mut(),
            Self::Op(evm) => evm.precompiles_mut(),
            Self::Tempo(_, _) => {
                panic!("precompiles_mut() not supported for Tempo variant")
            }
        }
    }

    fn inspector(&self) -> &Self::Inspector {
        match self {
            Self::Eth(evm) => evm.inspector(),
            Self::Op(evm) => evm.inspector(),
            Self::Tempo(evm, _) => &evm.inner.inner.inspector,
        }
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        match self {
            Self::Eth(evm) => evm.inspector_mut(),
            Self::Op(evm) => evm.inspector_mut(),
            Self::Tempo(evm, _) => &mut evm.inner.inner.inspector,
        }
    }

    fn enable_inspector(&mut self) {
        match self {
            Self::Eth(evm) => evm.enable_inspector(),
            Self::Op(evm) => evm.enable_inspector(),
            Self::Tempo(evm, _) => evm.inspect = true,
        }
    }

    fn disable_inspector(&mut self) {
        match self {
            Self::Eth(evm) => evm.disable_inspector(),
            Self::Op(evm) => evm.disable_inspector(),
            Self::Tempo(evm, _) => evm.inspect = false,
        }
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        match self {
            Self::Eth(evm) => evm.set_inspector_enabled(enabled),
            Self::Op(evm) => evm.set_inspector_enabled(enabled),
            Self::Tempo(evm, _) => evm.inspect = enabled,
        }
    }

    fn into_env(self) -> EvmEnv<Self::Spec>
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.into_env(),
            Self::Op(evm) => map_env(evm.into_env()),
            Self::Tempo(evm, _) => {
                let (_, env) = Evm::finish(evm);
                map_tempo_env(env)
            }
        }
    }

    fn transact(
        &mut self,
        tx: impl alloy_evm::IntoTxEnv<Self::Tx>,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            Self::Eth(evm) => {
                let eth = evm.transact(tx.into_tx_env().base);
                self.map_eth_result(eth)
            }
            Self::Op(evm) => evm.transact(tx),
            Self::Tempo(evm, _) => {
                // Convert OpTransaction<TxEnv> to TempoTxEnv
                let tempo_tx: TempoTxEnv = tx.into_tx_env().base.into();
                let result = Evm::transact_raw(evm, tempo_tx);
                self.map_tempo_result(result)
            }
        }
    }

    fn transact_commit(
        &mut self,
        tx: impl alloy_evm::IntoTxEnv<Self::Tx>,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error>
    where
        Self::DB: DatabaseCommit,
    {
        match self {
            Self::Eth(evm) => {
                let eth = evm.transact_commit(tx.into_tx_env().base);
                self.map_exec_result(eth)
            }
            Self::Op(evm) => evm.transact_commit(tx),
            Self::Tempo(evm, _) => {
                // Convert OpTransaction<TxEnv> to TempoTxEnv and transact with commit
                let tempo_tx: TempoTxEnv = tx.into_tx_env().base.into();
                let result = Evm::transact_raw(evm, tempo_tx);
                match result {
                    Ok(result_and_state) => {
                        evm.inner.inner.ctx.journaled_state.database.commit(result_and_state.state);
                        Ok(result_and_state.result.map_haltreason(map_tempo_halt_reason))
                    }
                    Err(e) => Err(self.map_tempo_err(e)),
                }
            }
        }
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            Self::Eth(evm) => {
                let res = evm.transact_raw(tx.base);
                self.map_eth_result(res)
            }
            Self::Op(evm) => evm.transact_raw(tx),
            Self::Tempo(evm, _) => {
                // Convert OpTransaction<TxEnv> to TempoTxEnv
                let tempo_tx: TempoTxEnv = tx.base.into();
                let result = Evm::transact_raw(evm, tempo_tx);
                self.map_tempo_result(result)
            }
        }
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            Self::Eth(evm) => {
                let eth = evm.transact_system_call(caller, contract, data);
                self.map_eth_result(eth)
            }
            Self::Op(evm) => evm.transact_system_call(caller, contract, data),
            Self::Tempo(evm, _) => {
                let result = Evm::transact_system_call(evm, caller, contract, data);
                self.map_tempo_result(result)
            }
        }
    }
}

/// Maps [`EvmEnv<TempoHardfork, TempoBlockEnv>`] to [`EvmEnv`].
#[cfg(feature = "tempo")]
fn map_tempo_env(env: EvmEnv<TempoHardfork, TempoBlockEnv>) -> EvmEnv {
    let eth_spec_id: SpecId = (*env.spec_id()).into();
    let cfg = env.cfg_env.with_spec(eth_spec_id);
    EvmEnv { cfg_env: cfg, block_env: env.block_env.inner }
}

/// Maps [`EvmEnv<OpSpecId>`] to [`EvmEnv`].
fn map_env(env: EvmEnv<OpSpecId>) -> EvmEnv {
    let eth_spec_id = env.spec_id().into_eth_spec();
    let cfg = env.cfg_env.with_spec(eth_spec_id);
    EvmEnv { cfg_env: cfg, block_env: env.block_env }
}
