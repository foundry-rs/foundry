use crate::eth::error::BlockchainError;
use alloy_evm::{eth::EthEvmContext, Database, EthEvm, Evm, EvmEnv};
use alloy_op_evm::OpEvm;
use alloy_primitives::{Address, Bytes};
use op_revm::{
    transaction::deposit::DepositTransactionParts, OpContext, OpHaltReason, OpTransaction,
    OpTransactionError,
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

type AnvilEvmResult<DBError> =
    Result<ResultAndState<OpHaltReason>, EVMError<DBError, OpTransactionError>>;

type AnvilExecResult<DBError> =
    Result<ExecutionResult<OpHaltReason>, EVMError<DBError, OpTransactionError>>;
pub enum EitherEvm<DB, I, P>
where
    DB: Database,
{
    Eth(EthEvm<DB, I, P>),
    Op(OpEvm<DB, I, P>),
}

impl<DB, I, P> EitherEvm<DB, I, P>
where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
    P: PrecompileProvider<EthEvmContext<DB>, Output = InterpreterResult>
        + PrecompileProvider<OpContext<DB>, Output = InterpreterResult>,
{
    pub fn transact_deposit(
        &mut self,
        tx: TxEnv,
        deposit: DepositTransactionParts,
    ) -> AnvilEvmResult<DB::Error> {
        match self {
            EitherEvm::Eth(_) => {
                return Err(EVMError::Custom(
                    BlockchainError::DepositTransactionUnsupported.to_string(),
                ));
            }
            EitherEvm::Op(evm) => {
                let op_tx = OpTransaction { base: tx, deposit, enveloped_tx: None };
                evm.transact_raw(op_tx)
            }
        }
    }

    pub fn transact_deposit_commit(
        &mut self,
        tx: TxEnv,
        deposit: DepositTransactionParts,
    ) -> AnvilExecResult<DB::Error>
    where
        DB: DatabaseCommit,
    {
        match self {
            EitherEvm::Eth(_) => {
                return Err(EVMError::Custom(
                    BlockchainError::DepositTransactionUnsupported.to_string(),
                ));
            }
            EitherEvm::Op(evm) => {
                let op_tx = OpTransaction { base: tx, deposit, enveloped_tx: None };
                evm.transact_commit(op_tx)
            }
        }
    }

    fn map_eth_result(
        &self,
        result: Result<ResultAndState<HaltReason>, EVMError<DB::Error>>,
    ) -> AnvilEvmResult<DB::Error> {
        match result {
            Ok(result) => {
                // Map the halt reason
                Ok(result.map_haltreason(|hr| OpHaltReason::Base(hr)))
            }
            Err(e) => {
                // Map the TransactionError
                match e {
                    EVMError::Transaction(invalid_tx) => {
                        Err(EVMError::Transaction(OpTransactionError::Base(invalid_tx)))
                    }
                    EVMError::Database(e) => Err(EVMError::Database(e)),
                    EVMError::Header(e) => Err(EVMError::Header(e)),
                    EVMError::Custom(e) => Err(EVMError::Custom(e)),
                }
            }
        }
    }

    fn map_exec_result(
        &self,
        result: Result<ExecutionResult, EVMError<DB::Error>>,
    ) -> AnvilExecResult<DB::Error> {
        match result {
            Ok(result) => {
                // Map the halt reason
                Ok(result.map_haltreason(|hr| OpHaltReason::Base(hr)))
            }
            Err(e) => {
                // Map the TransactionError
                match e {
                    EVMError::Transaction(invalid_tx) => {
                        Err(EVMError::Transaction(OpTransactionError::Base(invalid_tx)))
                    }
                    EVMError::Database(e) => Err(EVMError::Database(e)),
                    EVMError::Header(e) => Err(EVMError::Header(e)),
                    EVMError::Custom(e) => Err(EVMError::Custom(e)),
                }
            }
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
            EitherEvm::Eth(evm) => evm.block(),
            EitherEvm::Op(evm) => evm.block(),
        }
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        match self {
            EitherEvm::Eth(evm) => evm.db_mut(),
            EitherEvm::Op(evm) => evm.db_mut(),
        }
    }

    fn into_db(self) -> Self::DB
    where
        Self: Sized,
    {
        match self {
            EitherEvm::Eth(evm) => evm.into_db(),
            EitherEvm::Op(evm) => evm.into_db(),
        }
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        match self {
            EitherEvm::Eth(evm) => evm.finish(),
            EitherEvm::Op(evm) => {
                let (db, env) = evm.finish();
                // Convert the OpSpecId to EthSpecId
                let eth_spec_id = env.spec_id().into_eth_spec();
                let cfg = env.cfg_env.with_spec(eth_spec_id);

                (db, EvmEnv { cfg_env: cfg, block_env: env.block_env })
            }
        }
    }

    fn enable_inspector(&mut self) {
        match self {
            EitherEvm::Eth(evm) => evm.enable_inspector(),
            EitherEvm::Op(evm) => evm.enable_inspector(),
        }
    }

    fn disable_inspector(&mut self) {
        match self {
            EitherEvm::Eth(evm) => evm.disable_inspector(),
            EitherEvm::Op(evm) => evm.disable_inspector(),
        }
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        match self {
            EitherEvm::Eth(evm) => evm.set_inspector_enabled(enabled),
            EitherEvm::Op(evm) => evm.set_inspector_enabled(enabled),
        }
    }

    fn into_env(self) -> EvmEnv<Self::Spec>
    where
        Self: Sized,
    {
        match self {
            EitherEvm::Eth(evm) => evm.into_env(),
            EitherEvm::Op(evm) => {
                let env = evm.into_env();
                let eth_spec_id = env.spec_id().into_eth_spec();
                let cfg = env.cfg_env.with_spec(eth_spec_id);
                EvmEnv { cfg_env: cfg, block_env: env.block_env }
            }
        }
    }

    fn transact(
        &mut self,
        tx: impl alloy_evm::IntoTxEnv<Self::Tx>,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        match self {
            EitherEvm::Eth(evm) => {
                let eth = evm.transact(tx);
                self.map_eth_result(eth)
            }
            EitherEvm::Op(evm) => {
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
            EitherEvm::Eth(evm) => {
                let eth = evm.transact_commit(tx);
                self.map_exec_result(eth)
            }
            EitherEvm::Op(evm) => {
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
            EitherEvm::Eth(evm) => {
                let eth = evm.transact_system_call(caller, contract, data);
                self.map_eth_result(eth)
            }
            EitherEvm::Op(evm) => evm.transact_system_call(caller, contract, data),
        }
    }
}
