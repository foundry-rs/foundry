//! Ethereum execution using EVM2-owned state.

use alloy_consensus::transaction::Recovered;
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::state::StateOverride;
use evm2::{
    Evm, ExecutionConfig, Inspector, Precompiles, SpecId,
    bytecode::Bytecode,
    env::BlockEnvExt,
    ethereum::{TxEnvelope, ethereum_tx_registry},
    evm::{
        AccountInfo, Cache, DbResult, DynDatabase, EmptyDB, State, StateChangeSink, SystemTx,
        TxResult,
        bal::{Bal, BlockAccessIndex},
        registry::{HandlerError, HandlerResult},
    },
};
use foundry_evm_core::ethereum::{
    EthereumFork, FoundryContext, FoundryEvmTypes, RpcDatabase, SavedState,
};
use std::{collections::BTreeMap, convert::Infallible, mem, sync::Arc};

/// Ethereum's accepted execution state, independent of a persistent engine instance.
///
/// An engine is constructed for each execution. Parallel speculative calls share immutable
/// accepted state; committing execution transfers ownership into the engine and back out.
#[derive(Clone)]
pub struct Executor<D> {
    backing: Backing<D>,
    state: Option<SavedState>,
    config: ExecutionConfig<FoundryEvmTypes>,
    spec: SpecId,
    block: BlockEnvExt,
    context: FoundryContext,
}

#[derive(Clone)]
enum Backing<D> {
    Original(D),
    Fork(RpcDatabase),
    Empty(EmptyDB),
}

impl<D: DynDatabase> Backing<D> {
    fn database(&mut self) -> &mut dyn DynDatabase {
        match self {
            Self::Original(database) => database,
            Self::Fork(database) => database,
            Self::Empty(database) => database,
        }
    }
}

impl<D: DynDatabase> DynDatabase for Backing<D> {
    fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
        self.database().get_account(address)
    }
    fn get_code_by_hash(&mut self, hash: &B256) -> DbResult<Bytecode> {
        self.database().get_code_by_hash(hash)
    }
    fn get_storage(&mut self, address: &Address, key: &U256) -> DbResult<U256> {
        self.database().get_storage(address, key)
    }
    fn get_block_hash(&mut self, number: &U256) -> DbResult<B256> {
        self.database().get_block_hash(number)
    }
}

impl<D: DynDatabase + Clone + 'static> Executor<D> {
    fn save(&mut self, state: State<'static>) {
        if let Some(database) = state.initial().downcast_ref::<RpcDatabase>() {
            self.backing = Backing::Fork(database.clone());
        } else if let Some(database) = state.initial().downcast_ref::<EmptyDB>() {
            self.backing = Backing::Empty(*database);
        }
        self.state = Some(SavedState::capture(state));
    }

    /// Creates an executor over a backing database and explicit transaction configuration.
    pub fn new(
        backing: D,
        spec: SpecId,
        config: ExecutionConfig<FoundryEvmTypes>,
        block: BlockEnvExt,
    ) -> Self {
        let state = Some(SavedState::capture(State::new(backing.clone())));
        Self {
            backing: Backing::Original(backing),
            state,
            config,
            spec,
            block,
            context: FoundryContext::default(),
        }
    }

    /// Sets the live opcode environment used for subsequent execution.
    pub const fn set_context(&mut self, context: FoundryContext) {
        self.context = context;
    }

    /// Changes the block environment between native executions.
    pub const fn set_block(&mut self, block: BlockEnvExt) {
        self.block = block;
    }

    /// Returns the live block environment, including changes made during execution.
    pub const fn block(&self) -> &BlockEnvExt {
        &self.block
    }

    /// Returns accepted state entries without starting or changing an execution session.
    pub fn accepted_cache(&self) -> &Cache {
        self.state.as_ref().expect("execution session was invalidated").accepted_cache()
    }

    /// Sets an account balance between executions without creating a transaction.
    pub fn set_balance(&mut self, address: Address, balance: U256) -> DbResult<()> {
        let mut state = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        let result = state.overlay_db_mut().get_account(&address).map(|info| {
            let mut info = info.unwrap_or_default();
            info.balance = balance;
            state.overlay_db_mut().insert_account_info(&address, info);
        });
        self.save(state);
        result
    }

    /// Reads accepted storage without publishing read-cache changes.
    pub fn storage(&self, address: Address, slot: U256) -> DbResult<U256> {
        self.state
            .as_ref()
            .expect("execution session was invalidated")
            .speculate(self.backing.clone())
            .storage_slot_untracked(&address, &slot)
    }

    /// Reads accepted account metadata without publishing read-cache changes.
    pub fn account_info(&self, address: Address) -> DbResult<Option<AccountInfo>> {
        self.state
            .as_ref()
            .expect("execution session was invalidated")
            .speculate(self.backing.clone())
            .account_info_untracked(&address)
    }

    /// Reads accepted bytecode without publishing read-cache changes.
    pub fn code(&self, address: Address) -> DbResult<Bytecode> {
        let mut state = self
            .state
            .as_ref()
            .expect("execution session was invalidated")
            .speculate(self.backing.clone());
        let db = state.overlay_db_mut();
        match db.get_account(&address)? {
            Some(info) => match info.code {
                Some(code) => Ok(code),
                None => db.get_code_by_hash(&info.code_hash),
            },
            None => Ok(Bytecode::default()),
        }
    }

    /// Applies eth_call overrides to this execution session's accepted state.
    pub fn apply_state_overrides(&mut self, overrides: StateOverride) -> eyre::Result<()> {
        let mut state = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        let result = (|| {
            for (address, overrides) in overrides {
                let db = state.overlay_db_mut();
                let mut account = db.get_account(&address)?.unwrap_or_default();
                if let Some(balance) = overrides.balance {
                    account.balance = balance;
                }
                if let Some(nonce) = overrides.nonce {
                    account.nonce = nonce;
                }
                if let Some(code) = overrides.code {
                    account.set_code(Bytecode::new_raw_checked(code)?);
                }
                db.insert_account_info(&address, account);
                if let Some(storage) = overrides.state {
                    let cached = db.cache.storage.entry(address).or_default();
                    cached.wipe();
                    cached.slots.extend(storage.into_iter().map(|(key, value)| {
                        (U256::from_be_bytes(key.0), U256::from_be_bytes(value.0))
                    }));
                }
                if let Some(storage) = overrides.state_diff {
                    for (key, value) in storage {
                        db.insert_account_storage(&address, &key.into(), &value.into());
                    }
                }
            }
            Ok(())
        })();
        self.save(state);
        result
    }

    /// Executes a transaction and accepts its finalized state, including normal failure outcomes.
    ///
    /// Database/fatal execution errors and Rust panics invalidate this executor. Ownership return
    /// does not make a failed execution session reusable; callers must dispose of it.
    pub fn transact(
        &mut self,
        tx: &Recovered<TxEnvelope>,
    ) -> HandlerResult<TxResult<FoundryEvmTypes>> {
        let mut evm = self.engine();
        *evm.ext_mut() = self.context;
        evm.ext_mut().transaction_origin = Some(tx.signer());
        evm.ext_mut().origin_override = None;
        *evm.state_mut() = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        let result = evm.transact(tx).map(|executed| executed.commit());
        if !matches!(&result, Err(HandlerError::Database(_) | HandlerError::Fatal(_))) {
            self.block = *evm.block();
            self.context = *evm.ext();
            self.context.origin_override = None;
            let state = mem::replace(evm.state_mut(), State::new(EmptyDB::default()));
            self.save(state);
        }
        result
    }

    /// Executes without publishing state, on success, EVM revert or handler error.
    pub fn call(&self, tx: &Recovered<TxEnvelope>) -> HandlerResult<TxResult<FoundryEvmTypes>> {
        let mut evm = self.engine();
        *evm.ext_mut() = self.context;
        evm.ext_mut().transaction_origin = Some(tx.signer());
        evm.ext_mut().origin_override = None;
        *evm.state_mut() = self
            .state
            .as_ref()
            .expect("execution session was invalidated")
            .speculate(self.backing.clone());
        evm.call_tx(tx)
    }

    /// Commits execution and returns the owned inspector even when the handler returns an error.
    /// Database/fatal errors invalidate the execution session despite returning its inspector.
    pub fn inspect_transact<I: Inspector<FoundryEvmTypes> + 'static>(
        &mut self,
        tx: &Recovered<TxEnvelope>,
        inspector: I,
    ) -> (HandlerResult<TxResult<FoundryEvmTypes>>, I) {
        let mut evm = self.engine();
        *evm.ext_mut() = self.context;
        evm.ext_mut().transaction_origin = Some(tx.signer());
        evm.ext_mut().origin_override = None;
        *evm.state_mut() = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        evm.set_inspector(inspector);
        let result = evm.transact(tx).map(|executed| executed.commit());
        let inspector = *evm.clear_inspector_as::<I>().expect("installed inspector type changed");
        if !matches!(&result, Err(HandlerError::Database(_) | HandlerError::Fatal(_))) {
            self.block = *evm.block();
            self.context = *evm.ext();
            self.context.origin_override = None;
            let state = mem::replace(evm.state_mut(), State::new(EmptyDB::default()));
            self.save(state);
        }
        (result, inspector)
    }

    /// Streams finalized transaction changes before committing, returning the owned inspector.
    /// Database/fatal errors invalidate the execution session despite returning its inspector.
    pub fn inspect_transact_with_sink<
        I: Inspector<FoundryEvmTypes> + 'static,
        S: StateChangeSink<Error = Infallible>,
    >(
        &mut self,
        tx: &Recovered<TxEnvelope>,
        inspector: I,
        sink: &mut S,
    ) -> (HandlerResult<TxResult<FoundryEvmTypes>>, I) {
        let mut evm = self.engine();
        *evm.ext_mut() = self.context;
        evm.ext_mut().transaction_origin = Some(tx.signer());
        evm.ext_mut().origin_override = None;
        *evm.state_mut() = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        evm.set_inspector(inspector);
        let result = evm.transact(tx).map(|executed| match executed.commit_with(sink) {
            Ok(result) => result,
            Err(error) => match error {},
        });
        let inspector = *evm.clear_inspector_as::<I>().expect("installed inspector type changed");
        if !matches!(&result, Err(HandlerError::Database(_) | HandlerError::Fatal(_))) {
            self.block = *evm.block();
            self.context = *evm.ext();
            self.context.origin_override = None;
            let state = mem::replace(evm.state_mut(), State::new(EmptyDB::default()));
            self.save(state);
        }
        (result, inspector)
    }

    /// Executes speculatively and returns observations without publishing inspector mutations.
    pub fn inspect_call<I: Inspector<FoundryEvmTypes> + 'static>(
        &self,
        tx: &Recovered<TxEnvelope>,
        inspector: I,
    ) -> (HandlerResult<TxResult<FoundryEvmTypes>>, I) {
        let mut evm = self.engine();
        *evm.ext_mut() = self.context;
        evm.ext_mut().transaction_origin = Some(tx.signer());
        evm.ext_mut().origin_override = None;
        *evm.state_mut() = self
            .state
            .as_ref()
            .expect("execution session was invalidated")
            .speculate(self.backing.clone());
        evm.set_inspector(inspector);
        let result = evm.call_tx(tx);
        let inspector = *evm.clear_inspector_as::<I>().expect("installed inspector type changed");
        (result, inspector)
    }

    /// Accepts a protocol system call without ordinary transaction nonce or fee accounting.
    pub fn system_call(&mut self, tx: SystemTx) -> HandlerResult<TxResult<FoundryEvmTypes>> {
        let mut evm = self.engine();
        *evm.ext_mut() = self.context;
        evm.ext_mut().transaction_origin = Some(tx.caller);
        evm.ext_mut().origin_override = None;
        *evm.state_mut() = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        let result = evm.system_call(tx).map(|executed| executed.commit());
        if !matches!(&result, Err(HandlerError::Database(_) | HandlerError::Fatal(_))) {
            let state = mem::replace(evm.state_mut(), State::new(EmptyDB::default()));
            self.save(state);
        }
        result
    }

    /// Positions canonical BAL reads immediately before the target transaction.
    pub fn set_bal(&mut self, bal: Arc<Bal>, index: BlockAccessIndex) {
        let mut state = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        state.set_bal(bal);
        state.set_bal_index(index);
        state.set_allow_bal_db_fallback(true);
        self.save(state);
    }

    /// Applies account and slot data returned by the prestate tracer without inventing accounts.
    pub fn apply_prestate(
        &mut self,
        prestate: BTreeMap<Address, alloy_rpc_types::trace::geth::AccountState>,
    ) -> eyre::Result<()> {
        let mut state = self
            .state
            .take()
            .expect("execution session was invalidated")
            .resume(self.backing.clone());
        let result = (|| {
            for (address, account) in prestate {
                let code = Bytecode::new_raw_checked(account.code.unwrap_or_default())?;
                let info = evm2::evm::AccountInfo::default()
                    .with_balance(account.balance.unwrap_or_default())
                    .with_nonce(account.nonce.unwrap_or_default())
                    .with_code(code);
                let db = state.overlay_db_mut();
                if self.spec >= SpecId::SPURIOUS_DRAGON && info.is_empty() {
                    db.cache.accounts.insert(address, None);
                } else {
                    db.insert_account_info(&address, info);
                }
                for (slot, value) in account.storage {
                    db.insert_account_storage(&address, &slot.into(), &value.into());
                }
            }
            Ok(())
        })();
        self.save(state);
        result
    }

    fn engine<'a>(&self) -> Evm<'a, FoundryEvmTypes> {
        Evm::new_with_execution_config(
            self.config.clone(),
            self.spec,
            self.block,
            ethereum_tx_registry(self.spec),
            EmptyDB::default(),
            Precompiles::base(self.spec),
        )
    }
}

impl Executor<EmptyDB> {
    /// Starts a native session at a resolved RPC source with explicit execution configuration.
    pub fn from_fork(fork: &EthereumFork, config: ExecutionConfig<FoundryEvmTypes>) -> Self {
        let mut executor = Self::new(EmptyDB::default(), fork.env.spec, config, fork.env.block);
        executor.save(State::new(fork.database.clone()));
        executor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TxLegacy;
    use alloy_primitives::{Bytes, TxKind};
    use alloy_rpc_types::state::AccountOverride;
    use alloy_sol_types::SolCall;
    use evm2::{
        DatabaseError, EvmFeatures, Version,
        evm::InMemoryDB,
        interpreter::{Interpreter, Message, MessageResult},
    };
    use foundry_cheatcodes::{Vm, ethereum::Cheatcodes};
    use foundry_evm_core::constants::CHEATCODE_ADDRESS;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    const CALLER: Address = Address::with_last_byte(0xca);
    const CONTRACT: Address = Address::with_last_byte(0xc0);

    fn executor(code: impl AsRef<[u8]>) -> Executor<InMemoryDB> {
        let spec = SpecId::CANCUN;
        let mut version = Version::new(spec);
        version.features.remove(
            EvmFeatures::NONCE_CHECK | EvmFeatures::EIP3607 | EvmFeatures::BLOCK_GAS_LIMIT_CHECK,
        );
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            &CONTRACT,
            AccountInfo::default()
                .with_code(Bytecode::new_legacy(Bytes::copy_from_slice(code.as_ref()))),
        );
        Executor::new(
            db,
            spec,
            ExecutionConfig::for_spec_and_version(spec, version),
            BlockEnvExt::default(),
        )
    }

    fn tx(to: TxKind, input: Bytes) -> Recovered<TxEnvelope> {
        Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy { to, input, gas_limit: 1_000_000, ..Default::default() }),
            CALLER,
        )
    }

    #[test]
    fn system_writes_are_accepted_without_charging_the_system_sender() {
        // Store CALLER and ORIGIN, then increment the same slot in ordinary execution.
        let mut executor = executor([0x33, 0x5f, 0x55, 0x32, 0x60, 1, 0x55, 0x00]);
        let caller = evm2::evm::SYSTEM_ADDRESS;
        let before = executor
            .state
            .as_ref()
            .unwrap()
            .speculate(executor.backing.clone())
            .overlay_db_mut()
            .get_account(&caller)
            .unwrap();
        assert!(executor.system_call(SystemTx::new(CONTRACT, Bytes::new())).unwrap().status);
        let expected = U256::from_be_slice(caller.as_slice());
        assert_eq!(executor.storage(CONTRACT, U256::ZERO).unwrap(), expected);
        assert_eq!(executor.storage(CONTRACT, U256::ONE).unwrap(), expected);
        let after = executor
            .state
            .as_ref()
            .unwrap()
            .speculate(executor.backing.clone())
            .overlay_db_mut()
            .get_account(&caller)
            .unwrap();
        assert_eq!(before, after);
        // An ordinary execution must reset the system origin and preserve its accepted writes.
        assert!(executor.transact(&tx(TxKind::Call(CONTRACT), Bytes::new())).unwrap().status);
        assert_eq!(
            executor.storage(CONTRACT, U256::ONE).unwrap(),
            U256::from_be_slice(CALLER.as_slice())
        );
    }

    #[test]
    fn committed_writes_survive_and_speculative_writes_do_not_publish() {
        // Increment slot zero, return its new value.
        let mut executor = executor([
            0x5f, 0x54, 0x60, 1, 0x01, 0x80, 0x5f, 0x55, 0x5f, 0x52, 0x60, 32, 0x5f, 0xf3,
        ]);
        let tx = tx(TxKind::Call(CONTRACT), Bytes::new());
        assert_eq!(U256::from_be_slice(&executor.call(&tx).unwrap().output), U256::from(1));
        assert_eq!(U256::from_be_slice(&executor.call(&tx).unwrap().output), U256::from(1));
        assert_eq!(U256::from_be_slice(&executor.transact(&tx).unwrap().output), U256::from(1));
        assert_eq!(U256::from_be_slice(&executor.call(&tx).unwrap().output), U256::from(2));
        assert_eq!(U256::from_be_slice(&executor.transact(&tx).unwrap().output), U256::from(2));
    }

    #[test]
    fn reverting_writes_do_not_survive_committing_execution() {
        let mut executor = executor([0x60, 9, 0x5f, 0x55, 0x5f, 0x5f, 0xfd]);
        let result = executor.transact(&tx(TxKind::Call(CONTRACT), Bytes::new())).unwrap();
        assert!(!result.status);
        let state = executor.state.as_ref().unwrap();
        assert_eq!(
            state
                .speculate(executor.backing.clone())
                .storage_slot_untracked(&CONTRACT, &U256::ZERO)
                .unwrap(),
            U256::ZERO
        );
    }

    #[test]
    fn parallel_speculative_calls_share_accepted_state_without_publishing() {
        let executor = executor([
            0x5f, 0x54, 0x60, 1, 0x01, 0x80, 0x5f, 0x55, 0x5f, 0x52, 0x60, 32, 0x5f, 0xf3,
        ]);
        let tx = tx(TxKind::Call(CONTRACT), Bytes::new());
        std::thread::scope(|scope| {
            let first = scope.spawn(|| executor.call(&tx).unwrap());
            let second = scope.spawn(|| executor.call(&tx).unwrap());
            assert_eq!(U256::from_be_slice(&first.join().unwrap().output), U256::from(1));
            assert_eq!(U256::from_be_slice(&second.join().unwrap().output), U256::from(1));
        });
        assert_eq!(U256::from_be_slice(&executor.call(&tx).unwrap().output), U256::from(1));
    }

    #[test]
    fn deployment_persists_code_for_later_execution() {
        let mut executor = executor([]);
        // Deploy runtime that returns 42.
        let init = Bytes::from_static(&[
            0x60, 10, 0x60, 12, 0x60, 0, 0x39, 0x60, 10, 0x60, 0, 0xf3, 0x60, 42, 0x60, 0, 0x52,
            0x60, 32, 0x60, 0, 0xf3,
        ]);
        let deployed = executor.transact(&tx(TxKind::Create, init)).unwrap();
        assert!(deployed.status);
        let address = deployed.created_address.unwrap();
        let result = executor.call(&tx(TxKind::Call(address), Bytes::new())).unwrap();
        assert!(result.status);
        assert_eq!(U256::from_be_slice(&result.output), U256::from(42));
        assert_eq!(
            executor.code(address).unwrap().original_bytes(),
            Bytes::from_static(&[0x60, 42, 0x60, 0, 0x52, 0x60, 32, 0x60, 0, 0xf3,])
        );
    }

    #[test]
    fn state_overrides_wipe_unlisted_backing_slots_while_diff_preserves_them() {
        let mut executor = executor([0]);
        let Backing::Original(backing) = &mut executor.backing else { unreachable!() };
        backing.insert_account_storage(&CONTRACT, &U256::ZERO, &U256::from(5));
        backing.insert_account_storage(&CONTRACT, &U256::ONE, &U256::from(7));
        let slot = B256::ZERO;
        let value = B256::from(U256::from(9));
        let diff = AccountOverride {
            state_diff: Some([(slot, value)].into_iter().collect()),
            ..Default::default()
        };
        executor.apply_state_overrides([(CONTRACT, diff)].into_iter().collect()).unwrap();
        assert_eq!(executor.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(9));
        assert_eq!(executor.storage(CONTRACT, U256::ONE).unwrap(), U256::from(7));
        let state = AccountOverride {
            state: Some([(slot, value)].into_iter().collect()),
            ..Default::default()
        };
        executor.apply_state_overrides([(CONTRACT, state)].into_iter().collect()).unwrap();
        assert_eq!(executor.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(9));
        assert_eq!(executor.storage(CONTRACT, U256::ONE).unwrap(), U256::ZERO);
    }

    #[derive(Default)]
    struct Observations(Vec<&'static str>);

    impl Inspector<FoundryEvmTypes> for Observations {
        fn call(
            &mut self,
            _: &mut Interpreter<'_, '_, FoundryEvmTypes>,
            _: &mut Message<FoundryEvmTypes>,
        ) -> Option<MessageResult<FoundryEvmTypes>> {
            self.0.push("call");
            None
        }

        fn call_end(
            &mut self,
            _: &mut Interpreter<'_, '_, FoundryEvmTypes>,
            _: &Message<FoundryEvmTypes>,
            _: &mut MessageResult<FoundryEvmTypes>,
        ) {
            self.0.push("call_end");
        }
    }

    #[test]
    fn inspected_execution_returns_ordered_observations() {
        let mut executor = executor([0x00]);
        let tx = tx(TxKind::Call(CONTRACT), Bytes::new());
        let (result, inspector) = executor.inspect_call(&tx, Observations::default());
        assert!(result.unwrap().status);
        assert_eq!(inspector.0, ["call", "call_end"]);
        let (result, inspector) = executor.inspect_transact(&tx, Observations::default());
        assert!(result.unwrap().status);
        assert_eq!(inspector.0, ["call", "call_end"]);
    }

    #[derive(Clone)]
    struct FailingBacking(Backing<InMemoryDB>);

    impl DynDatabase for FailingBacking {
        fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
            if *address == Address::with_last_byte(0xee) {
                return Err(DatabaseError::new(std::io::Error::other("account read failed"), true));
            }
            self.0.get_account(address)
        }

        fn get_code_by_hash(&mut self, hash: &B256) -> DbResult<Bytecode> {
            self.0.get_code_by_hash(hash)
        }

        fn get_storage(&mut self, address: &Address, key: &U256) -> DbResult<U256> {
            self.0.get_storage(address, key)
        }

        fn get_block_hash(&mut self, number: &U256) -> DbResult<B256> {
            self.0.get_block_hash(number)
        }
    }

    #[test]
    fn handler_error_after_write_returns_inspector_and_disposes_session() {
        // Write slot zero, then fail a backing read through BALANCE.
        let base = executor([0x60, 9, 0x5f, 0x55, 0x60, 0xee, 0x31, 0x50, 0x00]);
        let mut executor =
            Executor::new(FailingBacking(base.backing), base.spec, base.config, base.block);
        let accepted = executor.clone();
        let tx = tx(TxKind::Call(CONTRACT), Bytes::new());
        let (result, inspector) = executor.inspect_transact(&tx, Observations::default());
        assert!(matches!(result, Err(HandlerError::Database(_))));
        assert_eq!(inspector.0, ["call"]);
        assert!(executor.state.is_none(), "execution error must dispose of the session");
        assert_eq!(accepted.storage(CONTRACT, U256::ZERO).unwrap(), U256::ZERO);
    }

    fn vm_program(input: &[u8], opcode: u8) -> Vec<u8> {
        vm_call_program(input, &[opcode, 0x5f, 0x52, 0x60, 32, 0x5f, 0xf3])
    }

    fn vm_call_program(input: &[u8], continuation: &[u8]) -> Vec<u8> {
        let size = u16::try_from(input.len()).unwrap().to_be_bytes();
        let mut code = vec![
            0x61, size[0], size[1], 0x61, 0, 0, 0x5f, 0x39, 0x5f, 0x5f, 0x61, size[0], size[1],
            0x5f, 0x5f, 0x73,
        ];
        code.extend_from_slice(CHEATCODE_ADDRESS.as_slice());
        // CALL and discard the success flag before the parent continuation.
        code.extend_from_slice(&[0x5a, 0xf1, 0x50]);
        code.extend_from_slice(continuation);
        let offset = u16::try_from(code.len()).unwrap().to_be_bytes();
        code[4..6].copy_from_slice(&offset);
        code.extend_from_slice(input);
        code
    }

    #[test]
    fn cheatcode_environment_changes_reach_the_running_parent() {
        let cases = [
            (Vm::warpCall { newTimestamp: U256::from(123) }.abi_encode(), 0x42, 123),
            (Vm::chainIdCall { newChainId: U256::from(99) }.abi_encode(), 0x46, 99),
            (Vm::txGasPriceCall { newGasPrice: U256::from(7) }.abi_encode(), 0x3a, 7),
            (Vm::feeCall { newBasefee: U256::from(9) }.abi_encode(), 0x48, 9),
        ];
        for (input, opcode, expected) in cases {
            let executor = executor(vm_program(&input, opcode));
            let (result, _) = executor
                .inspect_call(&tx(TxKind::Call(CONTRACT), Bytes::new()), Cheatcodes::default());
            let result = result.unwrap();
            assert!(result.status);
            assert_eq!(U256::from_be_slice(&result.output), U256::from(expected));
        }
    }

    #[test]
    fn cheatcode_environment_persists_only_for_committing_execution() {
        let mut executor = executor([0x46, 0x5f, 0x52, 0x60, 32, 0x5f, 0xf3]);
        let change = tx(
            TxKind::Call(CHEATCODE_ADDRESS),
            Vm::chainIdCall { newChainId: U256::from(99) }.abi_encode().into(),
        );
        let read = tx(TxKind::Call(CONTRACT), Bytes::new());
        let original = U256::from_be_slice(&executor.call(&read).unwrap().output);
        let (result, _) = executor.inspect_call(&change, Cheatcodes::default());
        assert!(result.unwrap().status);
        assert_eq!(U256::from_be_slice(&executor.call(&read).unwrap().output), original);
        let (result, _) = executor.inspect_transact(&change, Cheatcodes::default());
        assert!(result.unwrap().status);
        assert_eq!(U256::from_be_slice(&executor.call(&read).unwrap().output), U256::from(99));
    }

    #[test]
    fn cheatcode_storage_writes_obey_commit_and_discard() {
        let mut executor = executor([0x5f, 0x54, 0x5f, 0x52, 0x60, 32, 0x5f, 0xf3]);
        let write = tx(
            TxKind::Call(CHEATCODE_ADDRESS),
            Vm::storeCall { target: CONTRACT, slot: B256::ZERO, value: B256::from(U256::from(17)) }
                .abi_encode()
                .into(),
        );
        let read = tx(TxKind::Call(CONTRACT), Bytes::new());
        let (result, _) = executor.inspect_call(&write, Cheatcodes::default());
        assert!(result.unwrap().status);
        assert_eq!(U256::from_be_slice(&executor.call(&read).unwrap().output), U256::ZERO);
        let (result, _) = executor.inspect_transact(&write, Cheatcodes::default());
        assert!(result.unwrap().status);
        assert_eq!(U256::from_be_slice(&executor.call(&read).unwrap().output), U256::from(17));
    }

    #[test]
    fn cheatcode_backing_failure_is_an_execution_error() {
        let base = executor([]);
        let mut executor =
            Executor::new(FailingBacking(base.backing), base.spec, base.config, base.block);
        let call = tx(
            TxKind::Call(CHEATCODE_ADDRESS),
            Vm::dealCall { account: Address::with_last_byte(0xee), newBalance: U256::from(10) }
                .abi_encode()
                .into(),
        );
        let (result, _) = executor.inspect_transact(&call, Cheatcodes::default());
        assert!(matches!(result, Err(HandlerError::Database(_))));
        assert!(executor.state.is_none(), "cheatcode database failure must dispose of the session");
    }

    #[test]
    fn single_prank_changes_child_caller_and_origin_then_restores_parent() {
        let child = Address::with_last_byte(0xc1);
        let caller = Address::with_last_byte(0xb1);
        let origin = Address::with_last_byte(0xb2);
        let input = Vm::prank_1Call { msgSender: caller, txOrigin: origin }.abi_encode();
        let mut continuation = Vec::new();
        // Two sibling calls: the first consumes the prank, the second uses the real caller.
        for offset in [0, 64] {
            continuation.extend_from_slice(&[0x60, 64, 0x60, offset, 0x5f, 0x5f, 0x5f, 0x73]);
            continuation.extend_from_slice(child.as_slice());
            continuation.extend_from_slice(&[0x5a, 0xf1, 0x50]);
        }
        continuation.extend_from_slice(&[0x32, 0x60, 128, 0x52, 0x60, 160, 0x5f, 0xf3]);
        let mut executor = executor(vm_call_program(&input, &continuation));
        let Backing::Original(backing) = &mut executor.backing else { unreachable!() };
        backing.insert_account_info(
            &child,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[
                0x33, 0x5f, 0x52, 0x32, 0x60, 32, 0x52, 0x60, 64, 0x5f, 0xf3,
            ]))),
        );
        let (result, _) =
            executor.inspect_call(&tx(TxKind::Call(CONTRACT), Bytes::new()), Cheatcodes::default());
        let result = result.unwrap();
        assert!(result.status);
        let words = result
            .output
            .as_chunks::<32>()
            .0
            .iter()
            .map(|word| U256::from_be_slice(word))
            .collect::<Vec<_>>();
        assert_eq!(
            words,
            [
                U256::from_be_slice(caller.as_slice()),
                U256::from_be_slice(origin.as_slice()),
                U256::from_be_slice(CONTRACT.as_slice()),
                U256::from_be_slice(CALLER.as_slice()),
                U256::from_be_slice(CALLER.as_slice())
            ]
        );
    }

    #[derive(Default)]
    struct FaultAfterWrite {
        panic: bool,
        events: Observations,
    }

    impl Inspector<FoundryEvmTypes> for FaultAfterWrite {
        fn step(&mut self, interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
            if interp.opcode() == evm2::interpreter::op::BALANCE {
                assert!(!self.panic, "fault injection after write");
                let _ = interp.fail(evm2::ExecutionError::Fatal(evm2::AnyError::new(
                    std::io::Error::other("fatal injection after write"),
                )));
            }
        }

        fn call(
            &mut self,
            _: &mut Interpreter<'_, '_, FoundryEvmTypes>,
            _: &mut Message<FoundryEvmTypes>,
        ) -> Option<MessageResult<FoundryEvmTypes>> {
            self.events.0.push("call");
            None
        }

        fn call_end(
            &mut self,
            _: &mut Interpreter<'_, '_, FoundryEvmTypes>,
            _: &Message<FoundryEvmTypes>,
            _: &mut MessageResult<FoundryEvmTypes>,
        ) {
            self.events.0.push("call_end");
        }
    }

    #[test]
    fn fatal_error_after_write_returns_inspector_without_fabricating_end_events() {
        let mut executor = executor([0x60, 9, 0x5f, 0x55, 0x60, 0xee, 0x31, 0x50, 0x00]);
        let accepted = executor.clone();
        let request = tx(TxKind::Call(CONTRACT), Bytes::new());
        let (result, inspector) = executor.inspect_transact(&request, FaultAfterWrite::default());
        assert!(matches!(result, Err(HandlerError::Fatal(_))));
        assert_eq!(inspector.events.0, ["call"]);
        assert!(executor.state.is_none(), "fatal execution must dispose of the session");
        assert_eq!(accepted.storage(CONTRACT, U256::ZERO).unwrap(), U256::ZERO);
    }

    #[test]
    fn panic_after_write_disposes_session_without_publishing() {
        let mut executor = executor([0x60, 9, 0x5f, 0x55, 0x60, 0xee, 0x31, 0x50, 0x00]);
        let accepted = executor.clone();
        let request = tx(TxKind::Call(CONTRACT), Bytes::new());
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = executor
                .inspect_transact(&request, FaultAfterWrite { panic: true, ..Default::default() });
        }));
        assert!(result.is_err());
        assert!(executor.state.is_none(), "panic must dispose of the session");
        assert_eq!(accepted.storage(CONTRACT, U256::ZERO).unwrap(), U256::ZERO);
    }

    #[test]
    fn validation_error_preserves_reusable_accepted_state() {
        let mut executor = executor([0x60, 9, 0x5f, 0x55, 0x00]);
        let mut request = tx(TxKind::Call(CONTRACT), Bytes::new());
        let TxEnvelope::Legacy(inner) = request.inner_mut() else { unreachable!() };
        inner.gas_limit = 0;
        let (result, inspector) = executor.inspect_transact(&request, Observations::default());
        assert!(matches!(result, Err(HandlerError::IntrinsicGasTooLow { .. })));
        assert!(inspector.0.is_empty());
        assert!(executor.state.is_some());
        assert_eq!(executor.storage(CONTRACT, U256::ZERO).unwrap(), U256::ZERO);
        let request = tx(TxKind::Call(CONTRACT), Bytes::new());
        let (result, _) = executor.inspect_transact(&request, Observations::default());
        assert!(result.unwrap().status);
        assert_eq!(executor.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(9));
    }

    #[test]
    fn nested_database_error_escapes_catchable_calls_and_disposes_isolated_sessions() {
        let child = Address::with_last_byte(0xc1);
        let mut code = vec![0x5f, 0x5f, 0x5f, 0x5f, 0x5f, 0x73];
        code.extend_from_slice(child.as_slice());
        // Catch/discard the call success flag, as a Solidity low-level call would do.
        code.extend_from_slice(&[0x5a, 0xf1, 0x50, 0x00]);
        // Mutate the live environment before the nested post-write database failure.
        let code =
            vm_call_program(&Vm::warpCall { newTimestamp: U256::from(123) }.abi_encode(), &code);
        for isolate in [false, true] {
            let base = executor(&code);
            let mut executor =
                Executor::new(FailingBacking(base.backing), base.spec, base.config, base.block);
            executor
                .apply_state_overrides(
                    [(
                        child,
                        AccountOverride {
                            code: Some(Bytes::from_static(&[
                                0x60, 9, 0x5f, 0x55, 0x60, 0xee, 0x31, 0x50, 0x00,
                            ])),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                )
                .unwrap();
            let accepted = executor.clone();
            let cheats = Cheatcodes::new(Arc::new(foundry_cheatcodes::CheatsConfig {
                isolate,
                ..Default::default()
            }));
            let request = tx(TxKind::Call(CONTRACT), Bytes::new());
            let (result, _) = executor.inspect_call(&request, cheats.clone());
            assert!(matches!(result, Err(HandlerError::Database(_))));
            assert!(executor.state.is_some(), "speculation must not invalidate accepted state");
            assert_eq!(executor.block().timestamp, accepted.block().timestamp);
            assert_eq!(executor.context.transaction_origin, accepted.context.transaction_origin);
            assert_eq!(executor.storage(child, U256::ZERO).unwrap(), U256::ZERO);
            let (result, _) = executor.inspect_transact(&request, cheats);
            assert!(matches!(result, Err(HandlerError::Database(_))));
            assert!(
                executor.state.is_none(),
                "nested execution errors invalidate the owned session"
            );
            assert_eq!(accepted.storage(child, U256::ZERO).unwrap(), U256::ZERO);
        }
    }

    #[test]
    fn isolated_frame_validation_failures_remain_catchable() {
        let child = Address::with_last_byte(0xc1);
        for isolate in [false, true] {
            for (value, nonce, expected) in [(1, 0, 0), (0, u64::MAX, u8::from(!isolate))] {
                let mut code = vec![0x5f, 0x5f, 0x5f, 0x5f, 0x60, value, 0x73];
                code.extend_from_slice(child.as_slice());
                // Forward 100000 gas, then return the call success flag.
                code.extend_from_slice(&[
                    0x62, 1, 0x86, 0xa0, 0xf1, 0x5f, 0x52, 0x60, 32, 0x5f, 0xf3,
                ]);
                let mut executor = executor(code);
                executor
                    .apply_state_overrides(
                        [(CONTRACT, AccountOverride { nonce: Some(nonce), ..Default::default() })]
                            .into_iter()
                            .collect(),
                    )
                    .unwrap();
                let cheats = Cheatcodes::new(Arc::new(foundry_cheatcodes::CheatsConfig {
                    isolate,
                    ..Default::default()
                }));
                let request = tx(TxKind::Call(CONTRACT), Bytes::new());
                let (result, _) = executor.inspect_transact(&request, cheats);
                let result = result.unwrap();
                assert!(result.status, "frame validation failure escaped its caller");
                assert_eq!(U256::from_be_slice(&result.output), U256::from(expected));
                assert!(executor.state.is_some());
                assert_eq!(
                    executor.account_info(child).unwrap().map_or(U256::ZERO, |a| a.balance),
                    U256::ZERO
                );
            }
        }
    }
}
