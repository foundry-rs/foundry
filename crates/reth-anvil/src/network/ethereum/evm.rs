//! Ethereum execution with free blob calls and retained fork execution metadata.

use crate::fork::ForkInfo;
use alloy_consensus::TxType;
use alloy_evm::{
    Database, Evm, EvmEnv, EvmFactory,
    eth::{EthEvmContext, EthEvmFactory},
    precompiles::PrecompilesMap,
};
use alloy_primitives::{Address, Bytes, U256};
use revm::{
    Context, ExecuteEvm, InspectEvm, Inspector, SystemCallEvm,
    context::{BlockEnv, CfgEnv, DBErrorMarker, Evm as RevmEvm, JournalTr, TxEnv},
    context_interface::result::{EVMError, HaltReason, ResultAndState},
    handler::{EthFrame, Handler, instructions::EthInstructions, pre_execution, validation},
    inspector::{InspectorHandler, NoOpInspector},
    interpreter::{InitialAndFloorGas, interpreter::EthInterpreter},
    primitives::hardfork::SpecId,
};
use std::{fmt, marker::PhantomData, mem, sync::Arc};

/// Ethereum factory for anvil's call rules and fork execution metadata.
#[derive(Clone, Debug, Default)]
pub struct AnvilEthEvmFactory {
    fork: Option<Arc<dyn ForkInfo>>,
}

impl AnvilEthEvmFactory {
    /// Uses the fork's execution metadata while retaining consensus block numbers.
    pub const fn new(fork: Option<Arc<dyn ForkInfo>>) -> Self {
        Self { fork }
    }

    fn execution_env(&self, mut input: EvmEnv) -> (EvmEnv, Option<BlockEnv>) {
        let number = input.block_env.number.saturating_to();
        let consensus =
            self.fork.as_ref().and_then(|fork| fork.l1_block_number(number)).map(|l1| {
                let consensus = input.block_env.clone();
                input.block_env.number = U256::from(l1);
                consensus
            });
        (input, consensus)
    }
}

impl EvmFactory for AnvilEthEvmFactory {
    type Evm<DB: Database, I: Inspector<EthEvmContext<DB>>> = AnvilEthEvm<DB, I>;
    type Context<DB: Database> = EthEvmContext<DB>;
    type Tx = TxEnv;
    type Error<DBError: DBErrorMarker> = EVMError<DBError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;
    type Precompiles = PrecompilesMap;

    fn create_evm<DB: Database>(&self, db: DB, input: EvmEnv) -> Self::Evm<DB, NoOpInspector> {
        let (input, consensus) = self.execution_env(input);
        AnvilEthEvm {
            consensus,
            inner: EthEvmFactory::default().create_evm(db, input).into_inner(),
            inspect: false,
        }
    }

    fn create_evm_with_inspector<DB: Database, I: Inspector<Self::Context<DB>>>(
        &self,
        db: DB,
        input: EvmEnv,
        inspector: I,
    ) -> Self::Evm<DB, I> {
        let (input, consensus) = self.execution_env(input);
        AnvilEthEvm {
            consensus,
            inner: EthEvmFactory::default()
                .create_evm_with_inspector(db, input, inspector)
                .into_inner(),
            inspect: true,
        }
    }
}

type InnerEvm<DB, I> = RevmEvm<
    EthEvmContext<DB>,
    I,
    EthInstructions<EthInterpreter, EthEvmContext<DB>>,
    PrecompilesMap,
    EthFrame,
>;

/// Native Ethereum EVM with a handler override for zero-cap blob calls.
pub struct AnvilEthEvm<DB: Database, I> {
    inner: InnerEvm<DB, I>,
    /// The L2 environment used by block assembly; the interpreter sees the L1 number.
    consensus: Option<BlockEnv>,
    inspect: bool,
}

impl<DB: Database, I> fmt::Debug for AnvilEthEvm<DB, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilEthEvm").field("inspect", &self.inspect).finish_non_exhaustive()
    }
}

impl<DB: Database, I: Inspector<EthEvmContext<DB>>> Evm for AnvilEthEvm<DB, I> {
    type DB = DB;
    type Tx = TxEnv;
    type Error = EVMError<DB::Error>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type BlockEnv = BlockEnv;
    type Precompiles = PrecompilesMap;
    type Inspector = I;

    fn block(&self) -> &BlockEnv {
        self.consensus.as_ref().unwrap_or(&self.inner.ctx.block)
    }

    fn cfg_env(&self) -> &CfgEnv {
        &self.inner.ctx.cfg
    }

    fn chain_id(&self) -> u64 {
        self.inner.ctx.cfg.chain_id
    }

    fn transact_raw(&mut self, tx: TxEnv) -> Result<ResultAndState, Self::Error> {
        if self.inner.ctx.cfg.disable_base_fee
            && tx.tx_type == TxType::Eip4844 as u8
            && tx.max_fee_per_blob_gas == 0
        {
            self.inner.ctx.tx = tx;
            let mut handler = FreeBlobHandler(PhantomData);
            let result = if self.inspect {
                handler.inspect_run(&mut self.inner)
            } else {
                handler.run(&mut self.inner)
            };
            let state = self.inner.ctx.journaled_state.finalize();
            return Ok(ResultAndState { result: result?, state });
        }
        if self.inspect { self.inner.inspect_tx(tx) } else { self.inner.transact(tx) }
    }

    fn transact_system_call(
        &mut self,
        caller: Address,
        contract: Address,
        data: Bytes,
    ) -> Result<ResultAndState, Self::Error> {
        self.inner.system_call_with_caller(caller, contract, data)
    }

    fn finish(self) -> (DB, EvmEnv) {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.ctx;
        (
            journaled_state.database,
            EvmEnv { block_env: self.consensus.unwrap_or(block_env), cfg_env },
        )
    }

    fn set_inspector_enabled(&mut self, enabled: bool) {
        self.inspect = enabled;
    }

    fn components(&self) -> (&DB, &I, &PrecompilesMap) {
        (&self.inner.ctx.journaled_state.database, &self.inner.inspector, &self.inner.precompiles)
    }

    fn components_mut(&mut self) -> (&mut DB, &mut I, &mut PrecompilesMap) {
        (
            &mut self.inner.ctx.journaled_state.database,
            &mut self.inner.inspector,
            &mut self.inner.precompiles,
        )
    }
}

/// Skips only the blob fee validation and charge; execution retains BLOBBASEFEE.
struct FreeBlobHandler<DB, I>(PhantomData<(DB, I)>);

impl<DB: Database, I: Inspector<EthEvmContext<DB>>> Handler for FreeBlobHandler<DB, I> {
    type Evm = InnerEvm<DB, I>;
    type Error = EVMError<DB::Error>;
    type HaltReason = HaltReason;

    fn validate_env(&self, evm: &mut Self::Evm) -> Result<(), Self::Error> {
        with_zero_blob_fee(&mut evm.ctx, |ctx| validation::validate_env(ctx))
    }

    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
        _init_and_floor_gas: &mut InitialAndFloorGas,
    ) -> Result<(), Self::Error> {
        with_zero_blob_fee(&mut evm.ctx, pre_execution::validate_against_state_and_deduct_caller)
    }
}

impl<DB: Database, I: Inspector<EthEvmContext<DB>>> InspectorHandler for FreeBlobHandler<DB, I> {
    type IT = EthInterpreter;
}

/// Restores the execution price even when validation or fee deduction fails.
fn with_zero_blob_fee<DB: Database, T>(
    ctx: &mut EthEvmContext<DB>,
    f: impl FnOnce(&mut EthEvmContext<DB>) -> T,
) -> T {
    let price = ctx
        .block
        .blob_excess_gas_and_price
        .as_mut()
        .map(|blob| mem::replace(&mut blob.blob_gasprice, 0));
    let result = f(ctx);
    if let Some(price) = price
        && let Some(blob) = &mut ctx.block.blob_excess_gas_and_price
    {
        blob.blob_gasprice = price;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::eip4844::DATA_GAS_PER_BLOB;
    use alloy_primitives::{b256, bytes};
    use revm::{
        context_interface::{Block, result::InvalidTransaction},
        database::InMemoryDB,
        state::{AccountInfo, Bytecode},
    };

    #[test]
    fn free_blob_calls_preserve_the_price_and_clear_failed_state() {
        for inspect in [false, true] {
            let caller = Address::with_last_byte(0x42);
            let target = Address::with_last_byte(0x43);
            let mut db = InMemoryDB::default();
            db.insert_account_info(
                target,
                AccountInfo::from_bytecode(Bytecode::new_raw(bytes!("4a5f5260205ff3"))),
            );
            let mut cfg = CfgEnv::new_with_spec(SpecId::CANCUN);
            cfg.disable_base_fee = true;
            let mut block = BlockEnv::default();
            block.blob_excess_gas_and_price.as_mut().unwrap().blob_gasprice = 21;
            let mut evm = AnvilEthEvmFactory::default().create_evm(db, EvmEnv::new(cfg, block));
            evm.set_inspector_enabled(inspect);
            let tx = TxEnv::builder()
                .caller(caller)
                .call(target)
                .gas_limit(30_000)
                .gas_price(0)
                .gas_priority_fee(Some(0))
                .blob_hashes(vec![b256!(
                    "0100000000000000000000000000000000000000000000000000000000000000"
                )])
                .max_fee_per_blob_gas(0)
                .build()
                .unwrap();

            let free = evm.transact_raw(tx.clone()).unwrap();
            assert_eq!(U256::from_be_slice(free.result.output().unwrap()), U256::from(21));
            assert_eq!(free.state[&caller].info.balance, U256::ZERO);
            assert_eq!(evm.block().blob_gasprice(), Some(21));

            // A failed call must finalize its journal before the next transaction.
            let invalid = tx.clone().modify().gas_limit(1).build().unwrap();
            assert!(evm.transact_raw(invalid).is_err());
            assert_eq!(evm.block().blob_gasprice(), Some(21));
            assert!(evm.transact_raw(tx.clone()).unwrap().result.is_success());

            let priced = tx.modify().max_fee_per_blob_gas(30).build().unwrap();
            assert!(matches!(
                evm.transact_raw(priced.clone()),
                Err(EVMError::Transaction(InvalidTransaction::LackOfFundForMaxFee { .. }))
            ));
            let balance = U256::from(4_000_000);
            evm.db_mut().insert_account_info(caller, AccountInfo { balance, ..Default::default() });
            let paid = evm.transact_raw(priced).unwrap();
            assert_eq!(U256::from_be_slice(paid.result.output().unwrap()), U256::from(21));
            assert_eq!(
                paid.state[&caller].info.balance,
                balance - U256::from(21 * DATA_GAS_PER_BLOB)
            );
        }
    }
}
