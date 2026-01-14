use alloy_evm::{Database, EthEvm, Evm, EvmEnv, eth::EthEvmContext};
use alloy_monad_evm::{MonadContext, MonadEvm};
use alloy_op_evm::OpEvm;
use alloy_primitives::{Address, Bytes};
use monad_revm::MonadSpecId;
use op_revm::{OpContext, OpHaltReason, OpSpecId, OpTransaction, OpTransactionError};
use revm::{
    DatabaseCommit, Inspector,
    context::{
        BlockEnv, CfgEnv, TxEnv,
        result::{EVMError, ExecResultAndState, ExecutionResult, ResultAndState},
    },
    handler::PrecompileProvider,
    interpreter::InterpreterResult,
    primitives::hardfork::SpecId,
};

/// Alias for result type returned by [`Evm::transact`] methods.
type EitherEvmResult<DBError, HaltReason, TxError> =
    Result<ResultAndState<HaltReason>, EVMError<DBError, TxError>>;

/// Alias for result type returned by [`Evm::transact_commit`] methods.
type EitherExecResult<DBError, HaltReason, TxError> =
    Result<ExecutionResult<HaltReason>, EVMError<DBError, TxError>>;

/// [`EitherEvm`] delegates its calls to one of the two evm implementations; either [`EthEvm`] ,
/// [`OpEvm`] or [`MonadEvm`].
///
/// Calls are delegated to [`OpEvm`] only if optimism is enabled.
/// Calls are delegated to [`MonadEvm`] only if monad is enabled.
///
/// The call delegation is handled via its own implementation of the [`Evm`] trait.
///
/// The [`Evm::transact`] and other such calls work over the [`OpTransaction<TxEnv>`] type.
///
/// However, the [`Evm::HaltReason`] and [`Evm::Error`] leverage the optimism [`OpHaltReason`] and
/// [`OpTransactionError`] as these are supersets of the eth types. This makes it easier to map eth
/// types to op types and also prevents ignoring of any error that maybe thrown by [`OpEvm`].
///
/// TODO: MonadHaltReason and MonadTransactionError?
#[allow(clippy::large_enum_variant)]
pub enum EitherEvm<DB, I, P>
where
    DB: Database,
{
    /// [`EthEvm`] implementation.
    Eth(EthEvm<DB, I, P>),
    /// [`OpEvm`] implementation.
    Op(OpEvm<DB, I, P>),
    /// [`MonadEvm`] implementation.
    Monad(MonadEvm<DB, I, P>),
}

impl<DB, I, P> EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>> + Inspector<MonadContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<MonadContext<DB>, Output = InterpreterResult>,
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

    /// Converts the [`MonadEvm::transact`] result to [`EitherEvmResult`].
    ///
    /// Monad uses standard `HaltReason`, so we map it to `OpHaltReason::Base`.
    fn map_monad_result(
        &self,
        result: Result<ResultAndState, EVMError<DB::Error>>,
    ) -> EitherEvmResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => Ok(ResultAndState {
                result: result.result.map_haltreason(OpHaltReason::Base),
                state: result.state,
            }),
            Err(e) => Err(self.map_eth_err(e)),
        }
    }
}

impl<DB, I, P> Evm for EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>> + Inspector<MonadContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<MonadContext<DB>, Output = InterpreterResult>,
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
            Self::Monad(evm) => evm.block(),
        }
    }

    fn chain_id(&self) -> u64 {
        match self {
            Self::Eth(evm) => evm.chain_id(),
            Self::Op(evm) => evm.chain_id(),
            Self::Monad(evm) => evm.chain_id(),
        }
    }

    fn components(&self) -> (&Self::DB, &Self::Inspector, &Self::Precompiles) {
        match self {
            Self::Eth(evm) => evm.components(),
            Self::Op(evm) => evm.components(),
            Self::Monad(evm) => evm.components(),
        }
    }

    fn components_mut(&mut self) -> (&mut Self::DB, &mut Self::Inspector, &mut Self::Precompiles) {
        match self {
            Self::Eth(evm) => evm.components_mut(),
            Self::Op(evm) => evm.components_mut(),
            Self::Monad(evm) => evm.components_mut(),
        }
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        match self {
            Self::Eth(evm) => evm.db_mut(),
            Self::Op(evm) => evm.db_mut(),
            Self::Monad(evm) => evm.db_mut(),
        }
    }

    fn into_db(self) -> Self::DB
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.into_db(),
            Self::Op(evm) => evm.into_db(),
            Self::Monad(evm) => evm.into_db(),
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
                (db, map_op_env(env))
            }
            Self::Monad(evm) => {
                let (db, env) = evm.finish();
                (db, map_monad_env(env))
            }
        }
    }

    fn precompiles(&self) -> &Self::Precompiles {
        match self {
            Self::Eth(evm) => evm.precompiles(),
            Self::Op(evm) => evm.precompiles(),
            Self::Monad(evm) => evm.precompiles(),
        }
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        match self {
            Self::Eth(evm) => evm.precompiles_mut(),
            Self::Op(evm) => evm.precompiles_mut(),
            Self::Monad(evm) => evm.precompiles_mut(),
        }
    }

    fn inspector(&self) -> &Self::Inspector {
        match self {
            Self::Eth(evm) => evm.inspector(),
            Self::Op(evm) => evm.inspector(),
            Self::Monad(evm) => evm.inspector(),
        }
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        match self {
            Self::Eth(evm) => evm.inspector_mut(),
            Self::Op(evm) => evm.inspector_mut(),
            Self::Monad(evm) => evm.inspector_mut(),
        }
    }

    fn enable_inspector(&mut self) {
        match self {
            Self::Eth(evm) => evm.enable_inspector(),
            Self::Op(evm) => evm.enable_inspector(),
            Self::Monad(evm) => evm.enable_inspector(),
        }
    }

    fn disable_inspector(&mut self) {
        match self {
            Self::Eth(evm) => evm.disable_inspector(),
            Self::Op(evm) => evm.disable_inspector(),
            Self::Monad(evm) => evm.disable_inspector(),
        }
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        match self {
            Self::Eth(evm) => evm.set_inspector_enabled(enabled),
            Self::Op(evm) => evm.set_inspector_enabled(enabled),
            Self::Monad(evm) => evm.set_inspector_enabled(enabled),
        }
    }

    fn into_env(self) -> EvmEnv<Self::Spec>
    where
        Self: Sized,
    {
        match self {
            Self::Eth(evm) => evm.into_env(),
            Self::Op(evm) => map_op_env(evm.into_env()),
            Self::Monad(evm) => map_monad_env(evm.into_env()),
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
            Self::Monad(evm) => {
                let monad = evm.transact(tx.into_tx_env().base);
                self.map_monad_result(monad)
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
            Self::Monad(evm) => {
                let monad = evm.transact_commit(tx.into_tx_env().base);
                self.map_exec_result(monad)
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
            Self::Monad(evm) => {
                let res = evm.transact_raw(tx.base);
                self.map_monad_result(res)
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
            Self::Monad(evm) => {
                let monad = evm.transact_system_call(caller, contract, data);
                self.map_monad_result(monad)
            }
        }
    }
}

/// Maps [`EvmEnv<OpSpecId>`] to [`EvmEnv`].
fn map_op_env(env: EvmEnv<OpSpecId>) -> EvmEnv {
    let eth_spec_id = env.spec_id().into_eth_spec();
    let cfg = CfgEnv::new_with_spec(eth_spec_id).with_chain_id(env.cfg_env.chain_id);
    EvmEnv { cfg_env: cfg, block_env: env.block_env }
}

/// Maps [`EvmEnv<MonadSpecId>`] to [`EvmEnv`].
fn map_monad_env(env: EvmEnv<MonadSpecId>) -> EvmEnv {
    let eth_spec_id = env.spec_id().into_eth_spec();
    let cfg = CfgEnv::new_with_spec(eth_spec_id).with_chain_id(env.cfg_env.chain_id);
    EvmEnv { cfg_env: cfg, block_env: env.block_env }
}
