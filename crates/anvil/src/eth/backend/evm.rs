use crate::eth::error::BlockchainError;
use alloy_evm::{eth::EthEvmContext, Database, EthEvm, Evm, EvmEnv};
use alloy_op_evm::OpEvm;
use alloy_primitives::{Address, Bytes};
use op_revm::{
    transaction::deposit::DepositTransactionParts, OpContext, OpHaltReason, OpSpecId,
    OpTransaction, OpTransactionError,
};
use revm::{
    context::{
        result::{EVMError, ExecutionResult, HaltReason, ResultAndState},
        BlockEnv, TxEnv,
    },
    handler::PrecompileProvider,
    interpreter::InterpreterResult,
    primitives::hardfork::SpecId,
    DatabaseCommit, Inspector,
};

/// Alias for result type returned by [`Evm::transact`] methods.
type AnvilEvmResult<DBError, HaltReason, TxError> =
    Result<ResultAndState<HaltReason>, EVMError<DBError, TxError>>;

/// Alias for result type returned by [`Evm::transact_commit`] methods.
type AnvilExecResult<DBError, HaltReason, TxError> =
    Result<ExecutionResult<HaltReason>, EVMError<DBError, TxError>>;

/// [`EitherEvm`] delegates its calls to one of the two evm implementations; either [`EthEvm`] or
/// [`OpEvm`].
///
/// Calls are delegated to [`OpEvm`] only if the optimism is enabled.
///
/// The call delegation is handled via its own implementation of the [`Evm`] trait.
///
/// The [`Evm::transact`] and other such calls work over the [`TxEnv`] type. This is wrapped into
/// [`OpTransaction`] type in case of [`OpEvm`] under the hood.
///
/// However, the [`Evm::HaltReason`] and [`Evm::Error`] leverage the optimism [`OpHaltReason`] and
/// [`OpTransactionError`] as these are supersets of the eth types. This makes it easier to map eth
/// types to op types and also prevents ignoring of any error that maybe thrown by [`OpEvm`].
pub enum EitherEvm<DB, I, P>
where
    DB: Database,
{
    /// [`EthEvm`] implementation.
    Eth(EthEvm<DB, I, P>),
    /// [`OpEvm`] implementation.
    Op(OpEvm<DB, I, P>),
}

impl<DB, I, P> EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>,
{
    /// Transact a deposit transaction.
    ///
    /// This does not commit the transaction to the [`Database`].
    ///
    /// In order to transact and commit use [`EitherEvm::transact_deposit_commit`].
    pub fn transact_deposit(
        &mut self,
        tx: TxEnv,
        deposit: DepositTransactionParts,
    ) -> AnvilEvmResult<DB::Error, OpHaltReason, OpTransactionError> {
        match self {
            Self::Eth(_) => {
                Err(EVMError::Custom(BlockchainError::DepositTransactionUnsupported.to_string()))
            }
            Self::Op(evm) => {
                let op_tx = OpTransaction { base: tx, deposit, enveloped_tx: None };
                evm.transact_raw(op_tx)
            }
        }
    }

    /// Transact a deposit transaction and commits it to the [`Database`].
    pub fn transact_deposit_commit(
        &mut self,
        tx: TxEnv,
        deposit: DepositTransactionParts,
    ) -> AnvilExecResult<DB::Error, OpHaltReason, OpTransactionError>
    where
        DB: DatabaseCommit,
    {
        match self {
            Self::Eth(_) => {
                Err(EVMError::Custom(BlockchainError::DepositTransactionUnsupported.to_string()))
            }
            Self::Op(evm) => {
                let op_tx = OpTransaction { base: tx, deposit, enveloped_tx: None };
                evm.transact_commit(op_tx)
            }
        }
    }

    /// Converts the [`EthEvm::transact`] result to [`AnvilEvmResult`].
    fn map_eth_result(
        &self,
        result: Result<ResultAndState<HaltReason>, EVMError<DB::Error>>,
    ) -> AnvilEvmResult<DB::Error, OpHaltReason, OpTransactionError> {
        match result {
            Ok(result) => {
                // Map the halt reason
                Ok(result.map_haltreason(OpHaltReason::Base))
            }
            Err(e) => Err(self.map_eth_err(e)),
        }
    }

    /// Converts the [`EthEvm::transact_commit`] result to [`AnvilExecResult`].
    fn map_exec_result(
        &self,
        result: Result<ExecutionResult, EVMError<DB::Error>>,
    ) -> AnvilExecResult<DB::Error, OpHaltReason, OpTransactionError> {
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
    type Tx = TxEnv;
    type Spec = SpecId;

    fn block(&self) -> &BlockEnv {
        match self {
            Self::Eth(evm) => evm.block(),
            Self::Op(evm) => evm.block(),
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
                let eth = evm.transact(tx);
                self.map_eth_result(eth)
            }
            Self::Op(evm) => {
                let op_tx = OpTransaction::new(tx.into_tx_env());
                evm.transact(op_tx)
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
                let eth = evm.transact_commit(tx);
                self.map_exec_result(eth)
            }
            Self::Op(evm) => {
                let op_tx = OpTransaction::new(tx.into_tx_env());
                evm.transact_commit(op_tx)
            }
        }
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            Self::Eth(evm) => {
                let res = evm.transact_raw(tx);
                self.map_eth_result(res)
            }
            Self::Op(evm) => {
                let op_tx = OpTransaction::new(tx);
                evm.transact_raw(op_tx)
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
        }
    }
}

/// Maps [`EvmEnv<OpSpecId>`] to [`EvmEnv`].
fn map_env(env: EvmEnv<OpSpecId>) -> EvmEnv {
    let eth_spec_id = env.spec_id().into_eth_spec();
    let cfg = env.cfg_env.with_spec(eth_spec_id);
    EvmEnv { cfg_env: cfg, block_env: env.block_env }
}
