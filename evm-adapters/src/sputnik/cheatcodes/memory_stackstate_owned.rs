use sputnik::{
    backend::{Backend, Basic},
    executor::stack::{MemoryStackSubstate, StackState, StackSubstateMetadata},
    ExitError, Transfer,
};

use crate::{call_tracing::CallTraceArena, sputnik::cheatcodes::debugger::DebugArena};

use ethers::{
    abi::RawLog,
    types::{H160, H256, U256},
};

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[derive(Clone, Default)]
pub struct RecordAccess {
    pub reads: RefCell<BTreeMap<H160, Vec<H256>>>,
    pub writes: RefCell<BTreeMap<H160, Vec<H256>>>,
}

#[derive(Clone, Default, Debug)]
pub struct ExpectedEmit {
    pub depth: usize,
    pub log: Option<RawLog>,
    pub checks: [bool; 4],
    /// Whether this expected emit was actually found in the subcall
    pub found: bool,
}

#[derive(Clone, Default, Debug)]
pub struct Prank {
    /// Address of the contract that called prank
    pub prank_caller: H160,
    /// Address to set msg.sender to
    pub new_caller: H160,
    /// New origin to use
    pub new_origin: Option<H160>,
    /// Call depth at which the prank was called
    pub depth: usize,
}

/// This struct implementation is copied from [upstream](https://github.com/rust-blockchain/evm/blob/5ecf36ce393380a89c6f1b09ef79f686fe043624/src/executor/stack/state.rs#L412) and modified to own the Backend type.
///
/// We had to copy it so that we can modify the Stack's internal backend, because
/// the upstream MemoryStackState only has an immutable reference to `Backend` which
/// does not allow us to do so.
#[derive(Clone)]
pub struct MemoryStackStateOwned<'config, B> {
    pub backend: B,
    pub substate: MemoryStackSubstate<'config>,
    /// Tracing enabled
    pub trace_enabled: bool,
    /// Current call index used for incrementing traces index vec below
    pub call_index: usize,
    /// Temporary value used for putting logs in the correct trace
    pub trace_index: usize,
    /// Arena allocator that holds a tree of traces
    pub traces: Vec<CallTraceArena>,
    /// Expected revert storage of bytes
    pub expected_revert: Option<Vec<u8>>,
    /// Next call's prank
    pub next_prank: Option<Prank>,
    /// StartPrank information
    pub prank: Option<Prank>,
    /// List of accesses done during a call
    pub accesses: Option<RecordAccess>,
    /// All logs accumulated (regardless of revert status)
    pub all_logs: Vec<String>,
    /// Expected events by end of the next call
    pub expected_emits: Vec<ExpectedEmit>,
    pub mocked_calls: BTreeMap<H160, BTreeMap<Vec<u8>, Vec<u8>>>,
    pub expected_calls: BTreeMap<H160, Vec<Vec<u8>>>,
    /// Debug enabled
    pub debug_enabled: bool,
    /// An arena allocator of DebugNodes for debugging purposes
    pub debug_steps: Vec<DebugArena>,
    /// Instruction pointers that maps an address to a mapping of pc to ic
    pub debug_instruction_pointers: Dip,
}

impl<'config, B: Backend> MemoryStackStateOwned<'config, B> {
    pub fn deposit(&mut self, address: H160, value: U256) {
        self.substate.deposit(address, value, &self.backend);
    }

    pub fn increment_call_index(&mut self) {
        self.traces.push(Default::default());
        self.debug_steps.push(Default::default());
        self.call_index += 1;
    }
    pub fn trace_mut(&mut self) -> &mut CallTraceArena {
        &mut self.traces[self.call_index]
    }

    pub fn debug_mut(&mut self) -> &mut DebugArena {
        &mut self.debug_steps[self.call_index]
    }

    pub fn trace(&self) -> &CallTraceArena {
        &self.traces[self.call_index]
    }

    pub fn reset_traces(&mut self) {
        self.traces = vec![Default::default()];
        self.call_index = 0;
    }
}

/// Debug Instruction pointers: a tuple with 2 maps, the first being for creation
/// sourcemaps, the second for runtime sourcemaps.
///
/// Each has a structure of (Address => (program_counter => instruction_counter))
/// For sourcemap usage, we need to convert a program counter to an instruction counter and use the
/// instruction counter as the index into the sourcemap vector. An instruction counter (pointer) is
/// just the program counter minus the sum of push bytes (i.e. PUSH1(0x01), would apply a -1 effect
/// to all subsequent instruction counters)
pub type Dip =
    (BTreeMap<H160, Rc<BTreeMap<usize, usize>>>, BTreeMap<H160, Rc<BTreeMap<usize, usize>>>);

impl<'config, B: Backend> MemoryStackStateOwned<'config, B> {
    pub fn new(
        metadata: StackSubstateMetadata<'config>,
        backend: B,
        trace_enabled: bool,
        debug_enabled: bool,
    ) -> Self {
        Self {
            backend,
            substate: MemoryStackSubstate::new(metadata),
            trace_enabled,
            call_index: 0,
            trace_index: 1,
            traces: vec![Default::default()],
            expected_revert: None,
            next_prank: None,
            prank: None,
            accesses: None,
            all_logs: Default::default(),
            expected_emits: Default::default(),
            mocked_calls: Default::default(),
            expected_calls: Default::default(),
            debug_enabled,
            debug_steps: vec![Default::default()],
            debug_instruction_pointers: (BTreeMap::new(), BTreeMap::new()),
        }
    }
}

impl<'config, B: Backend> Backend for MemoryStackStateOwned<'config, B> {
    fn gas_price(&self) -> U256 {
        self.backend.gas_price()
    }
    fn origin(&self) -> H160 {
        self.backend.origin()
    }
    fn block_hash(&self, number: U256) -> H256 {
        self.backend.block_hash(number)
    }
    fn block_number(&self) -> U256 {
        self.backend.block_number()
    }
    fn block_coinbase(&self) -> H160 {
        self.backend.block_coinbase()
    }
    fn block_timestamp(&self) -> U256 {
        self.backend.block_timestamp()
    }
    fn block_difficulty(&self) -> U256 {
        self.backend.block_difficulty()
    }
    fn block_gas_limit(&self) -> U256 {
        self.backend.block_gas_limit()
    }
    fn block_base_fee_per_gas(&self) -> U256 {
        self.backend.block_base_fee_per_gas()
    }
    fn chain_id(&self) -> U256 {
        self.backend.chain_id()
    }

    fn exists(&self, address: H160) -> bool {
        self.substate.known_account(address).is_some() || self.backend.exists(address)
    }

    fn basic(&self, address: H160) -> Basic {
        self.substate.known_basic(address).unwrap_or_else(|| self.backend.basic(address))
    }

    fn code(&self, address: H160) -> Vec<u8> {
        self.substate.known_code(address).unwrap_or_else(|| self.backend.code(address))
    }

    fn storage(&self, address: H160, key: H256) -> H256 {
        if let Some(record_accesses) = &self.accesses {
            record_accesses.reads.borrow_mut().entry(address).or_insert_with(Vec::new).push(key);
        }
        self.substate
            .known_storage(address, key)
            .unwrap_or_else(|| self.backend.storage(address, key))
    }

    fn original_storage(&self, address: H160, key: H256) -> Option<H256> {
        if let Some(value) = self.substate.known_original_storage(address, key) {
            return Some(value)
        }

        self.backend.original_storage(address, key)
    }
}

impl<'config, B: Backend> StackState<'config> for MemoryStackStateOwned<'config, B> {
    fn metadata(&self) -> &StackSubstateMetadata<'config> {
        self.substate.metadata()
    }

    fn metadata_mut(&mut self) -> &mut StackSubstateMetadata<'config> {
        self.substate.metadata_mut()
    }

    fn enter(&mut self, gas_limit: u64, is_static: bool) {
        self.substate.enter(gas_limit, is_static)
    }

    fn exit_commit(&mut self) -> Result<(), ExitError> {
        self.substate.exit_commit()
    }

    fn exit_revert(&mut self) -> Result<(), ExitError> {
        self.substate.exit_revert()
    }

    fn exit_discard(&mut self) -> Result<(), ExitError> {
        self.substate.exit_discard()
    }

    fn is_empty(&self, address: H160) -> bool {
        if let Some(known_empty) = self.substate.known_empty(address) {
            return known_empty
        }

        self.backend.basic(address).balance == U256::zero() &&
            self.backend.basic(address).nonce == U256::zero() &&
            self.backend.code(address).len() == 0
    }

    fn deleted(&self, address: H160) -> bool {
        self.substate.deleted(address)
    }

    fn is_cold(&self, address: H160) -> bool {
        self.substate.is_cold(address)
    }

    fn is_storage_cold(&self, address: H160, key: H256) -> bool {
        self.substate.is_storage_cold(address, key)
    }

    fn inc_nonce(&mut self, address: H160) {
        self.substate.inc_nonce(address, &self.backend);
    }

    fn set_storage(&mut self, address: H160, key: H256, value: H256) {
        if let Some(record_accesses) = &self.accesses {
            record_accesses.writes.borrow_mut().entry(address).or_insert_with(Vec::new).push(key);
        }
        self.substate.set_storage(address, key, value)
    }

    fn reset_storage(&mut self, address: H160) {
        self.substate.reset_storage(address, &self.backend);
    }

    fn log(&mut self, address: H160, topics: Vec<H256>, data: Vec<u8>) {
        self.substate.log(address, topics, data);
    }

    fn set_deleted(&mut self, address: H160) {
        self.substate.set_deleted(address)
    }

    fn set_code(&mut self, address: H160, code: Vec<u8>) {
        self.substate.set_code(address, code, &self.backend)
    }

    fn transfer(&mut self, transfer: Transfer) -> Result<(), ExitError> {
        self.substate.transfer(transfer, &self.backend)
    }

    fn reset_balance(&mut self, address: H160) {
        self.substate.reset_balance(address, &self.backend)
    }

    fn touch(&mut self, address: H160) {
        self.substate.touch(address, &self.backend)
    }
}
