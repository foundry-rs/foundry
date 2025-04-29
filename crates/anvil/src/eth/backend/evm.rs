use crate::eth::error::BlockchainError;
use alloy_evm::{eth::EthEvmContext, evm, Database, EthEvm, Evm, EvmEnv, EvmFactory};
use alloy_op_evm::OpEvm;
use alloy_primitives::{Address, Bytes};
use foundry_evm::{backend::DatabaseError, Env};
use foundry_evm_core::evm::FoundryPrecompiles;
use op_revm::{
    precompiles::OpPrecompiles, transaction::deposit::DepositTransactionParts, L1BlockInfo,
    OpContext, OpHaltReason, OpSpecId, OpTransaction, OpTransactionError,
};
use revm::{
    context::{
        result::{EVMError, ExecutionResult, HaltReason, HaltReasonTr, ResultAndState},
        BlockEnv, Cfg, CfgEnv, ContextTr, Evm as RevmEvm, TxEnv,
    },
    handler::{instructions::EthInstructions, PrecompileProvider},
    interpreter::InterpreterResult,
    primitives::hardfork::SpecId,
    DatabaseCommit, Inspector, Journal,
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
    pub fn block(&self) -> &BlockEnv {
        match self {
            EitherEvm::Eth(evm) => evm.block(),
            EitherEvm::Op(evm) => evm.block(),
        }
    }

    pub fn transact_raw(
        &mut self,
        tx: TxEnv,
        deposit: Option<DepositTransactionParts>,
    ) -> AnvilEvmResult<DB::Error> {
        match self {
            Self::Eth(evm) => {
                if deposit.is_some() {
                    return Err(EVMError::Custom(
                        "Deposit transactions not supported via EthEvm".to_string(),
                    ));
                }
                let res = evm.transact_raw(tx);
                self.map_eth_result(res)
            }
            Self::Op(evm) => {
                let op_tx = OpTransaction {
                    base: tx,
                    deposit: deposit.unwrap_or_default(),
                    // Used to compute L1 gas cost
                    enveloped_tx: None,
                };
                evm.transact_raw(op_tx)
            }
        }
    }

    pub fn transact_commit(
        &mut self,
        tx: TxEnv,
        deposit: Option<DepositTransactionParts>,
    ) -> AnvilExecResult<DB::Error>
    where
        DB: DatabaseCommit,
    {
        match self {
            Self::Eth(evm) => {
                if deposit.is_some() {
                    return Err(EVMError::Custom(
                        "Deposit transactions not supported via EthEvm".to_string(),
                    ));
                }
                let res = evm.transact_commit(tx);
                self.map_exec_result(res)
            }
            Self::Op(evm) => {
                let op_tx = OpTransaction {
                    base: tx,
                    deposit: deposit.unwrap_or_default(),
                    // Used to compute L1 gas cost
                    enveloped_tx: None,
                };
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
