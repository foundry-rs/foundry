use self::{
    env::Broadcast,
    expect::{handle_expect_emit, handle_expect_revert, ExpectedCallType},
    mapping::MappingSlots,
    util::{check_if_fixed_gas_limit, process_create, BroadcastableTransactions, MAGIC_SKIP_BYTES},
};
use crate::{
    abi::HEVMCalls,
    executor::{
        backend::DatabaseExt, inspector::cheatcodes::env::RecordedLogs, CHEATCODE_ADDRESS,
        HARDHAT_CONSOLE_ADDRESS,
    },
    utils::{b160_to_h160, b256_to_h256, h160_to_b160, ru256_to_u256},
};
use ethers::{
    abi::{AbiDecode, AbiEncode, RawLog},
    signers::LocalWallet,
    types::{
        transaction::eip2718::TypedTransaction, Address, Bytes, NameOrAddress, TransactionRequest,
        U256,
    },
};
use foundry_common::evm::Breakpoints;
use foundry_utils::error::SolError;
use itertools::Itertools;
use revm::{
    interpreter::{opcode, CallInputs, CreateInputs, Gas, InstructionResult, Interpreter},
    primitives::{BlockEnv, TransactTo, B160, B256},
    EVMData, Inspector,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs::File,
    io::BufReader,
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

/// Cheatcodes related to the execution environment.
mod env;
pub use env::{Log, Prank, RecordAccess};
/// Assertion helpers (such as `expectEmit`)
mod expect;
pub use expect::{
    ExpectedCallData, ExpectedEmit, ExpectedRevert, MockCallDataContext, MockCallReturnData,
};

/// Cheatcodes that interact with the external environment (FFI etc.)
mod ext;
/// Fork related cheatcodes
mod fork;
/// File-system related cheatcodes
mod fs;
/// Cheatcodes that configure the fuzzer
mod fuzz;
/// Mapping related cheatcodes
mod mapping;
/// Snapshot related cheatcodes
mod snapshot;
/// Utility cheatcodes (`sign` etc.)
pub mod util;
pub use util::{BroadcastableTransaction, DEFAULT_CREATE2_DEPLOYER};

mod config;
use crate::executor::{backend::RevertDiagnostic, inspector::utils::get_create_address};
pub use config::CheatsConfig;

mod error;
pub(crate) use error::{bail, ensure, fmt_err};
pub use error::{Error, Result};

/// Tracks the expected calls per address.
/// For each address, we track the expected calls per call data. We track it in such manner
/// so that we don't mix together calldatas that only contain selectors and calldatas that contain
/// selector and arguments (partial and full matches).
/// This then allows us to customize the matching behavior for each call data on the
/// `ExpectedCallData` struct and track how many times we've actually seen the call on the second
/// element of the tuple.
pub type ExpectedCallTracker = BTreeMap<Address, BTreeMap<Vec<u8>, (ExpectedCallData, u64)>>;

/// An inspector that handles calls to various cheatcodes, each with their own behavior.
///
/// Cheatcodes can be called by contracts during execution to modify the VM environment, such as
/// mocking addresses, signatures and altering call reverts.
///
/// Executing cheatcodes can be very powerful. Most cheatcodes are limited to evm internals, but
/// there are also cheatcodes like `ffi` which can execute arbitrary commands or `writeFile` and
/// `readFile` which can manipulate files of the filesystem. Therefore, several restrictions are
/// implemented for these cheatcodes:
///
///    - `ffi`, and file cheatcodes are _always_ opt-in (via foundry config) and never enabled by
///      default: all respective cheatcode handlers implement the appropriate checks
///    - File cheatcodes require explicit permissions which paths are allowed for which operation,
///      see `Config.fs_permission`
///    - Only permitted accounts are allowed to execute cheatcodes in forking mode, this ensures no
///      contract deployed on the live network is able to execute cheatcodes by simply calling the
///      cheatcode address: by default, the caller, test contract and newly deployed contracts are
///      allowed to execute cheatcodes
#[derive(Clone, Debug, Default)]
pub struct Cheatcodes {
    /// The block environment
    ///
    /// Used in the cheatcode handler to overwrite the block environment separately from the
    /// execution block environment.
    pub block: Option<BlockEnv>,

    /// The gas price
    ///
    /// Used in the cheatcode handler to overwrite the gas price separately from the gas price
    /// in the execution environment.
    pub gas_price: Option<U256>,

    /// Address labels
    pub labels: BTreeMap<Address, String>,

    /// Rememebered private keys
    pub script_wallets: Vec<LocalWallet>,

    /// Whether the skip cheatcode was activated
    pub skip: bool,

    /// Prank information
    pub prank: Vec<Prank>,

    /// Expected revert information
    pub expected_revert: Option<ExpectedRevert>,

    /// Additional diagnostic for reverts
    pub fork_revert_diagnostic: Option<RevertDiagnostic>,

    /// Recorded storage reads and writes
    pub accesses: Option<RecordAccess>,

    /// Recorded logs
    pub recorded_logs: Option<RecordedLogs>,

    /// Mocked calls
    pub mocked_calls: BTreeMap<Address, BTreeMap<MockCallDataContext, MockCallReturnData>>,

    /// Expected calls
    pub expected_calls: ExpectedCallTracker,

    /// Expected emits
    pub expected_emits: VecDeque<ExpectedEmit>,

    /// Map of context depths to memory offset ranges that may be written to within the call depth.
    pub allowed_mem_writes: BTreeMap<u64, Vec<Range<u64>>>,

    /// Current broadcasting information
    pub broadcast: Option<Broadcast>,

    /// Used to correct the nonce of --sender after the initiating call. For more, check
    /// `docs/scripting`.
    pub corrected_nonce: bool,

    /// Scripting based transactions
    pub broadcastable_transactions: BroadcastableTransactions,

    /// Additional, user configurable context this Inspector has access to when inspecting a call
    pub config: Arc<CheatsConfig>,

    /// Test-scoped context holding data that needs to be reset every test run
    pub context: Context,

    // Commit FS changes such as file creations, writes and deletes.
    // Used to prevent duplicate changes file executing non-committing calls.
    pub fs_commit: bool,

    pub serialized_jsons: BTreeMap<String, BTreeMap<String, Value>>,

    /// Records all eth deals
    pub eth_deals: Vec<DealRecord>,

    /// Holds the stored gas info for when we pause gas metering. It is an `Option<Option<..>>`
    /// because the `call` callback in an `Inspector` doesn't get access to
    /// the `revm::Interpreter` which holds the `revm::Gas` struct that
    /// we need to copy. So we convert it to a `Some(None)` in `apply_cheatcode`, and once we have
    /// the interpreter, we copy the gas struct. Then each time there is an execution of an
    /// operation, we reset the gas.
    pub gas_metering: Option<Option<revm::interpreter::Gas>>,

    /// Holds stored gas info for when we pause gas metering, and we're entering/inside
    /// CREATE / CREATE2 frames. This is needed to make gas meter pausing work correctly when
    /// paused and creating new contracts.
    pub gas_metering_create: Option<Option<revm::interpreter::Gas>>,

    /// Holds mapping slots info
    pub mapping_slots: Option<BTreeMap<Address, MappingSlots>>,

    /// current program counter
    pub pc: usize,
    /// Breakpoints supplied by the `vm.breakpoint("<char>")` cheatcode
    /// char -> pc
    pub breakpoints: Breakpoints,
}

impl Cheatcodes {
    /// Creates a new `Cheatcodes` based on the given settings
    pub fn new(block: BlockEnv, gas_price: U256, config: CheatsConfig) -> Self {
        Self {
            corrected_nonce: false,
            block: Some(block),
            gas_price: Some(gas_price),
            config: Arc::new(config),
            fs_commit: true,
            ..Default::default()
        }
    }

    #[instrument(level = "error", name = "apply", target = "evm::cheatcodes", skip_all)]
    fn apply_cheatcode<DB: DatabaseExt>(
        &mut self,
        data: &mut EVMData<'_, DB>,
        caller: Address,
        call: &CallInputs,
    ) -> Result {
        // Decode the cheatcode call
        let decoded = HEVMCalls::decode(&call.input)?;

        // ensure the caller is allowed to execute cheatcodes,
        // but only if the backend is in forking mode
        data.db.ensure_cheatcode_access_forking_mode(caller)?;

        // TODO: Log the opcode for the debugger
        let opt = env::apply(self, data, caller, &decoded)
            .transpose()
            .or_else(|| util::apply(self, data, &decoded))
            .or_else(|| expect::apply(self, data, &decoded))
            .or_else(|| fuzz::apply(&decoded))
            .or_else(|| ext::apply(self, &decoded))
            .or_else(|| fs::apply(self, &decoded))
            .or_else(|| snapshot::apply(data, &decoded))
            .or_else(|| fork::apply(self, data, &decoded));
        match opt {
            Some(res) => res,
            None => Err(fmt_err!("Unhandled cheatcode: {decoded:?}. This is a bug.")),
        }
    }

    /// Determines the address of the contract and marks it as allowed
    ///
    /// There may be cheatcodes in the constructor of the new contract, in order to allow them
    /// automatically we need to determine the new address
    fn allow_cheatcodes_on_create<DB: DatabaseExt>(
        &self,
        data: &mut EVMData<'_, DB>,
        inputs: &CreateInputs,
    ) {
        let old_nonce = data
            .journaled_state
            .state
            .get(&inputs.caller)
            .map(|acc| acc.info.nonce)
            .unwrap_or_default();
        let created_address = get_create_address(inputs, old_nonce);

        if data.journaled_state.depth > 1 &&
            !data.db.has_cheatcode_access(b160_to_h160(inputs.caller))
        {
            // we only grant cheat code access for new contracts if the caller also has
            // cheatcode access and the new contract is created in top most call
            return
        }

        data.db.allow_cheatcode_access(created_address);
    }

    /// Called when there was a revert.
    ///
    /// Cleanup any previously applied cheatcodes that altered the state in such a way that revm's
    /// revert would run into issues.
    pub fn on_revert<DB: DatabaseExt>(&mut self, data: &mut EVMData<'_, DB>) {
        trace!(deals=?self.eth_deals.len(), "Rolling back deals");

        // Delay revert clean up until expected revert is handled, if set.
        if self.expected_revert.is_some() {
            return
        }

        // we only want to apply cleanup top level
        if data.journaled_state.depth() > 0 {
            return
        }

        // Roll back all previously applied deals
        // This will prevent overflow issues in revm's [`JournaledState::journal_revert`] routine
        // which rolls back any transfers.
        while let Some(record) = self.eth_deals.pop() {
            if let Some(acc) = data.journaled_state.state.get_mut(&h160_to_b160(record.address)) {
                acc.info.balance = record.old_balance.into();
            }
        }
    }
}

impl<DB> Inspector<DB> for Cheatcodes
where
    DB: DatabaseExt,
{
    fn initialize_interp(
        &mut self,
        _: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _: bool,
    ) -> InstructionResult {
        // When the first interpreter is initialized we've circumvented the balance and gas checks,
        // so we apply our actual block data with the correct fees and all.
        if let Some(block) = self.block.take() {
            data.env.block = block;
        }
        if let Some(gas_price) = self.gas_price.take() {
            data.env.tx.gas_price = gas_price.into();
        }

        InstructionResult::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _: bool,
    ) -> InstructionResult {
        self.pc = interpreter.program_counter();

        // reset gas if gas metering is turned off
        match self.gas_metering {
            Some(None) => {
                // need to store gas metering
                self.gas_metering = Some(Some(interpreter.gas));
            }
            Some(Some(gas)) => {
                match interpreter.contract.bytecode.bytecode()[interpreter.program_counter()] {
                    opcode::CREATE | opcode::CREATE2 => {
                        // set we're about to enter CREATE frame to meter its gas on first opcode
                        // inside it
                        self.gas_metering_create = Some(None)
                    }
                    opcode::STOP | opcode::RETURN | opcode::SELFDESTRUCT | opcode::REVERT => {
                        // If we are ending current execution frame, we want to just fully reset gas
                        // otherwise weird things with returning gas from a call happen
                        // ref: https://github.com/bluealloy/revm/blob/2cb991091d32330cfe085320891737186947ce5a/crates/revm/src/evm_impl.rs#L190
                        //
                        // It would be nice if we had access to the interpreter in `call_end`, as we
                        // could just do this there instead.
                        match self.gas_metering_create {
                            None | Some(None) => {
                                interpreter.gas = revm::interpreter::Gas::new(0);
                            }
                            Some(Some(gas)) => {
                                // If this was CREATE frame, set correct gas limit. This is needed
                                // because CREATE opcodes deduct additional gas for code storage,
                                // and deducted amount is compared to gas limit. If we set this to
                                // 0, the CREATE would fail with out of gas.
                                //
                                // If we however set gas limit to the limit of outer frame, it would
                                // cause a panic after erasing gas cost post-create. Reason for this
                                // is pre-create REVM records `gas_limit - (gas_limit / 64)` as gas
                                // used, and erases costs by `remaining` gas post-create.
                                // gas used ref: https://github.com/bluealloy/revm/blob/2cb991091d32330cfe085320891737186947ce5a/crates/revm/src/instructions/host.rs#L254-L258
                                // post-create erase ref: https://github.com/bluealloy/revm/blob/2cb991091d32330cfe085320891737186947ce5a/crates/revm/src/instructions/host.rs#L279
                                interpreter.gas = revm::interpreter::Gas::new(gas.limit());

                                // reset CREATE gas metering because we're about to exit its frame
                                self.gas_metering_create = None
                            }
                        }
                    }
                    _ => {
                        // if just starting with CREATE opcodes, record its inner frame gas
                        if let Some(None) = self.gas_metering_create {
                            self.gas_metering_create = Some(Some(interpreter.gas))
                        }

                        // dont monitor gas changes, keep it constant
                        interpreter.gas = gas;
                    }
                }
            }
            _ => {}
        }

        // Record writes and reads if `record` has been called
        if let Some(storage_accesses) = &mut self.accesses {
            match interpreter.contract.bytecode.bytecode()[interpreter.program_counter()] {
                opcode::SLOAD => {
                    let key = try_or_continue!(interpreter.stack().peek(0));
                    storage_accesses
                        .reads
                        .entry(b160_to_h160(interpreter.contract().address))
                        .or_insert_with(Vec::new)
                        .push(key.into());
                }
                opcode::SSTORE => {
                    let key = try_or_continue!(interpreter.stack().peek(0));

                    // An SSTORE does an SLOAD internally
                    storage_accesses
                        .reads
                        .entry(b160_to_h160(interpreter.contract().address))
                        .or_insert_with(Vec::new)
                        .push(key.into());
                    storage_accesses
                        .writes
                        .entry(b160_to_h160(interpreter.contract().address))
                        .or_insert_with(Vec::new)
                        .push(key.into());
                }
                _ => (),
            }
        }

        // If the allowed memory writes cheatcode is active at this context depth, check to see
        // if the current opcode can either mutate directly or expand memory. If the opcode at
        // the current program counter is a match, check if the modified memory lies within the
        // allowed ranges. If not, revert and fail the test.
        if let Some(ranges) = self.allowed_mem_writes.get(&data.journaled_state.depth()) {
            // The `mem_opcode_match` macro is used to match the current opcode against a list of
            // opcodes that can mutate memory (either directly or expansion via reading). If the
            // opcode is a match, the memory offsets that are being written to are checked to be
            // within the allowed ranges. If not, the test is failed and the transaction is
            // reverted. For all opcodes that can mutate memory aside from MSTORE,
            // MSTORE8, and MLOAD, the size and destination offset are on the stack, and
            // the macro expands all of these cases. For MSTORE, MSTORE8, and MLOAD, the
            // size of the memory write is implicit, so these cases are hard-coded.
            macro_rules! mem_opcode_match {
                ([$(($opcode:ident, $offset_depth:expr, $size_depth:expr, $writes:expr)),*]) => {
                    match interpreter.contract.bytecode.bytecode()[interpreter.program_counter()] {
                        ////////////////////////////////////////////////////////////////
                        //    OPERATIONS THAT CAN EXPAND/MUTATE MEMORY BY WRITING     //
                        ////////////////////////////////////////////////////////////////

                        opcode::MSTORE => {
                            // The offset of the mstore operation is at the top of the stack.
                            let offset = ru256_to_u256(try_or_continue!(interpreter.stack().peek(0))).as_u64();

                            // If none of the allowed ranges contain [offset, offset + 32), memory has been
                            // unexpectedly mutated.
                            if !ranges.iter().any(|range| {
                                range.contains(&offset) && range.contains(&(offset + 31))
                            }) {
                                revert_helper::disallowed_mem_write(offset, 32, interpreter, ranges);
                                return InstructionResult::Revert
                            }
                        }
                        opcode::MSTORE8 => {
                            // The offset of the mstore8 operation is at the top of the stack.
                            let offset = ru256_to_u256(try_or_continue!(interpreter.stack().peek(0))).as_u64();

                            // If none of the allowed ranges contain the offset, memory has been
                            // unexpectedly mutated.
                            if !ranges.iter().any(|range| range.contains(&offset)) {
                                revert_helper::disallowed_mem_write(offset, 1, interpreter, ranges);
                                return InstructionResult::Revert
                            }
                        }

                        ////////////////////////////////////////////////////////////////
                        //        OPERATIONS THAT CAN EXPAND MEMORY BY READING        //
                        ////////////////////////////////////////////////////////////////

                        opcode::MLOAD => {
                            // The offset of the mload operation is at the top of the stack
                            let offset = ru256_to_u256(try_or_continue!(interpreter.stack().peek(0))).as_u64();

                            // If the offset being loaded is >= than the memory size, the
                            // memory is being expanded. If none of the allowed ranges contain
                            // [offset, offset + 32), memory has been unexpectedly mutated.
                            if offset >= interpreter.memory.len() as u64 && !ranges.iter().any(|range| {
                                range.contains(&offset) && range.contains(&(offset + 31))
                            }) {
                                revert_helper::disallowed_mem_write(offset, 32, interpreter, ranges);
                                return InstructionResult::Revert
                            }
                        }

                        ////////////////////////////////////////////////////////////////
                        //          OPERATIONS WITH OFFSET AND SIZE ON STACK          //
                        ////////////////////////////////////////////////////////////////

                        $(opcode::$opcode => {
                            // The destination offset of the operation is at the top of the stack.
                            let dest_offset = ru256_to_u256(try_or_continue!(interpreter.stack().peek($offset_depth))).as_u64();

                            // The size of the data that will be copied is the third item on the stack.
                            let size = ru256_to_u256(try_or_continue!(interpreter.stack().peek($size_depth))).as_u64();

                            // If none of the allowed ranges contain [dest_offset, dest_offset + size),
                            // memory outside of the expected ranges has been touched. If the opcode
                            // only reads from memory, this is okay as long as the memory is not expanded.
                            let fail_cond = !ranges.iter().any(|range| {
                                    range.contains(&dest_offset) &&
                                        range.contains(&(dest_offset + size.saturating_sub(1)))
                                }) && ($writes ||
                                    [dest_offset, (dest_offset + size).saturating_sub(1)].into_iter().any(|offset| {
                                        offset >= interpreter.memory.len() as u64
                                    })
                                );

                            // If the failure condition is met, set the output buffer to a revert string
                            // that gives information about the allowed ranges and revert.
                            if fail_cond {
                                revert_helper::disallowed_mem_write(dest_offset, size, interpreter, ranges);
                                return InstructionResult::Revert
                            }
                        })*
                        _ => ()
                    }
                }
            }

            // Check if the current opcode can write to memory, and if so, check if the memory
            // being written to is registered as safe to modify.
            mem_opcode_match!([
                (CALLDATACOPY, 0, 2, true),
                (CODECOPY, 0, 2, true),
                (RETURNDATACOPY, 0, 2, true),
                (EXTCODECOPY, 1, 3, true),
                (CALL, 5, 6, true),
                (CALLCODE, 5, 6, true),
                (STATICCALL, 4, 5, true),
                (DELEGATECALL, 4, 5, true),
                (SHA3, 0, 1, false),
                (LOG0, 0, 1, false),
                (LOG1, 0, 1, false),
                (LOG2, 0, 1, false),
                (LOG3, 0, 1, false),
                (LOG4, 0, 1, false),
                (CREATE, 1, 2, false),
                (CREATE2, 1, 2, false),
                (RETURN, 0, 1, false),
                (REVERT, 0, 1, false)
            ])
        }

        // Record writes with sstore (and sha3) if `StartMappingRecording` has been called
        if let Some(mapping_slots) = &mut self.mapping_slots {
            mapping::on_evm_step(mapping_slots, interpreter, data)
        }

        InstructionResult::Continue
    }

    fn log(
        &mut self,
        _: &mut EVMData<'_, DB>,
        address: &B160,
        topics: &[B256],
        data: &bytes::Bytes,
    ) {
        if !self.expected_emits.is_empty() {
            handle_expect_emit(
                self,
                RawLog {
                    topics: topics.iter().copied().map(b256_to_h256).collect_vec(),
                    data: data.to_vec(),
                },
                &b160_to_h160(*address),
            );
        }

        // Stores this log if `recordLogs` has been called
        if let Some(storage_recorded_logs) = &mut self.recorded_logs {
            storage_recorded_logs.entries.push(Log {
                emitter: b160_to_h160(*address),
                inner: RawLog {
                    topics: topics.iter().copied().map(b256_to_h256).collect_vec(),
                    data: data.to_vec(),
                },
            });
        }
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        is_static: bool,
    ) -> (InstructionResult, Gas, bytes::Bytes) {
        if call.contract == h160_to_b160(CHEATCODE_ADDRESS) {
            let gas = Gas::new(call.gas_limit);
            match self.apply_cheatcode(data, b160_to_h160(call.context.caller), call) {
                Ok(retdata) => (InstructionResult::Return, gas, retdata.0),
                Err(err) => (InstructionResult::Revert, gas, err.encode_error().0),
            }
        } else if call.contract != h160_to_b160(HARDHAT_CONSOLE_ADDRESS) {
            // Handle expected calls

            // Grab the different calldatas expected.
            if let Some(expected_calls_for_target) =
                self.expected_calls.get_mut(&(b160_to_h160(call.contract)))
            {
                // Match every partial/full calldata
                for (calldata, (expected, actual_count)) in expected_calls_for_target.iter_mut() {
                    // Increment actual times seen if...
                    // The calldata is at most, as big as this call's input, and
                    if calldata.len() <= call.input.len() &&
                        // Both calldata match, taking the length of the assumed smaller one (which will have at least the selector), and
                        *calldata == call.input[..calldata.len()] &&
                        // The value matches, if provided
                        expected
                            .value
                            .map_or(true, |value| value == call.transfer.value.into()) &&
                        // The gas matches, if provided
                        expected.gas.map_or(true, |gas| gas == call.gas_limit) &&
                        // The minimum gas matches, if provided
                        expected.min_gas.map_or(true, |min_gas| min_gas <= call.gas_limit)
                    {
                        *actual_count += 1;
                    }
                }
            }

            // Handle mocked calls
            if let Some(mocks) = self.mocked_calls.get(&b160_to_h160(call.contract)) {
                let ctx = MockCallDataContext {
                    calldata: call.input.clone().into(),
                    value: Some(call.transfer.value.into()),
                };
                if let Some(mock_retdata) = mocks.get(&ctx) {
                    return (
                        mock_retdata.ret_type,
                        Gas::new(call.gas_limit),
                        mock_retdata.data.clone().0,
                    )
                } else if let Some((_, mock_retdata)) = mocks.iter().find(|(mock, _)| {
                    mock.calldata.len() <= call.input.len() &&
                        *mock.calldata == call.input[..mock.calldata.len()] &&
                        mock.value.map_or(true, |value| value == call.transfer.value.into())
                }) {
                    return (
                        mock_retdata.ret_type,
                        Gas::new(call.gas_limit),
                        mock_retdata.data.0.clone(),
                    )
                }
            }

            // Apply our prank
            if let Some(prank) = self.prank.last_mut() {
                if data.journaled_state.depth() >= prank.depth &&
                    call.context.caller == h160_to_b160(prank.prank_caller)
                {
                    let mut prank_applied = false;
                    // At the target depth we set `msg.sender`
                    if data.journaled_state.depth() == prank.depth {
                        call.context.caller = h160_to_b160(prank.new_caller);
                        call.transfer.source = h160_to_b160(prank.new_caller);
                        prank_applied = true;
                    }

                    // At the target depth, or deeper, we set `tx.origin`
                    if let Some(new_origin) = prank.new_origin {
                        data.env.tx.caller = h160_to_b160(new_origin);
                        prank_applied = true;
                    }

                    // If prank applied for first time, then update
                    if prank_applied {
                       prank.used = true; 
                    }
                }
            }

            // Apply our broadcast
            if let Some(broadcast) = &self.broadcast {
                // We only apply a broadcast *to a specific depth*.
                //
                // We do this because any subsequent contract calls *must* exist on chain and
                // we only want to grab *this* call, not internal ones
                if data.journaled_state.depth() == broadcast.depth &&
                    call.context.caller == h160_to_b160(broadcast.original_caller)
                {
                    // At the target depth we set `msg.sender` & tx.origin.
                    // We are simulating the caller as being an EOA, so *both* must be set to the
                    // broadcast.origin.
                    data.env.tx.caller = h160_to_b160(broadcast.new_origin);

                    call.context.caller = h160_to_b160(broadcast.new_origin);
                    call.transfer.source = h160_to_b160(broadcast.new_origin);
                    // Add a `legacy` transaction to the VecDeque. We use a legacy transaction here
                    // because we only need the from, to, value, and data. We can later change this
                    // into 1559, in the cli package, relatively easily once we
                    // know the target chain supports EIP-1559.
                    if !is_static {
                        if let Err(err) = data
                            .journaled_state
                            .load_account(h160_to_b160(broadcast.new_origin), data.db)
                        {
                            return (
                                InstructionResult::Revert,
                                Gas::new(call.gas_limit),
                                err.encode_string().0,
                            )
                        }

                        let is_fixed_gas_limit = check_if_fixed_gas_limit(data, call.gas_limit);

                        let account = data
                            .journaled_state
                            .state()
                            .get_mut(&h160_to_b160(broadcast.new_origin))
                            .unwrap();

                        self.broadcastable_transactions.push_back(BroadcastableTransaction {
                            rpc: data.db.active_fork_url(),
                            transaction: TypedTransaction::Legacy(TransactionRequest {
                                from: Some(broadcast.new_origin),
                                to: Some(NameOrAddress::Address(b160_to_h160(call.contract))),
                                value: Some(call.transfer.value.into()),
                                data: Some(call.input.clone().into()),
                                nonce: Some(account.info.nonce.into()),
                                gas: if is_fixed_gas_limit {
                                    Some(call.gas_limit.into())
                                } else {
                                    None
                                },
                                ..Default::default()
                            }),
                        });

                        // call_inner does not increase nonces, so we have to do it ourselves
                        account.info.nonce += 1;
                    } else if broadcast.single_call {
                        return (
                            InstructionResult::Revert,
                            Gas::new(0),
                            "Staticcalls are not allowed after vm.broadcast. Either remove it, or use vm.startBroadcast instead."
                            .to_string()
                            .encode()
                            .into()
                        );
                    }
                }
            }

            (InstructionResult::Continue, Gas::new(call.gas_limit), bytes::Bytes::new())
        } else {
            (InstructionResult::Continue, Gas::new(call.gas_limit), bytes::Bytes::new())
        }
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: InstructionResult,
        retdata: bytes::Bytes,
        _: bool,
    ) -> (InstructionResult, Gas, bytes::Bytes) {
        if call.contract == h160_to_b160(CHEATCODE_ADDRESS) ||
            call.contract == h160_to_b160(HARDHAT_CONSOLE_ADDRESS)
        {
            return (status, remaining_gas, retdata)
        }

        if data.journaled_state.depth() == 0 && self.skip {
            return (
                InstructionResult::Revert,
                remaining_gas,
                Error::custom_bytes(MAGIC_SKIP_BYTES).encode_error().0,
            )
        }

        // Clean up pranks
        if let Some(prank) = self.prank.last() {
            if data.journaled_state.depth() == prank.depth {
                data.env.tx.caller = h160_to_b160(prank.prank_origin);
            }
            if prank.single_call {
                let _ = self.prank.pop();
            }
        }

        // Clean up broadcast
        if let Some(broadcast) = &self.broadcast {
            if data.journaled_state.depth() == broadcast.depth {
                data.env.tx.caller = h160_to_b160(broadcast.original_origin);
            }

            if broadcast.single_call {
                std::mem::take(&mut self.broadcast);
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if data.journaled_state.depth() <= expected_revert.depth {
                let expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                return match handle_expect_revert(
                    false,
                    expected_revert.reason.as_ref(),
                    status,
                    retdata.into(),
                ) {
                    Err(error) => {
                        trace!(expected=?expected_revert, ?error, ?status, "Expected revert mismatch");
                        (InstructionResult::Revert, remaining_gas, error.encode_error().0)
                    }
                    Ok((_, retdata)) => (InstructionResult::Return, remaining_gas, retdata.0),
                }
            }
        }

        // At the end of the call,
        // we need to check if we've found all the emits.
        // We know we've found all the expected emits in the right order
        // if the queue is fully matched.
        // If it's not fully matched, then either:
        // 1. Not enough events were emitted (we'll know this because the amount of times we
        // inspected events will be less than the size of the queue) 2. The wrong events
        // were emitted (The inspected events should match the size of the queue, but still some
        // events will not be matched)

        // First, check that we're at the call depth where the emits were declared from.
        let should_check_emits = self
            .expected_emits
            .iter()
            .any(|expected| expected.depth == data.journaled_state.depth()) &&
            // Ignore staticcalls
            !call.is_static;
        // If so, check the emits
        if should_check_emits {
            // Not all emits were matched.
            if self.expected_emits.iter().any(|expected| !expected.found) {
                return (
                    InstructionResult::Revert,
                    remaining_gas,
                    "Log != expected log".to_string().encode().into(),
                )
            } else {
                // All emits were found, we're good.
                // Clear the queue, as we expect the user to declare more events for the next call
                // if they wanna match further events.
                self.expected_emits.clear()
            }
        }

        // If the depth is 0, then this is the root call terminating
        if data.journaled_state.depth() == 0 {
            // Match expected calls
            for (address, calldatas) in &self.expected_calls {
                // Loop over each address, and for each address, loop over each calldata it expects.
                for (calldata, (expected, actual_count)) in calldatas {
                    // Grab the values we expect to see
                    let ExpectedCallData { gas, min_gas, value, count, call_type } = expected;
                    let calldata = Bytes::from(calldata.clone());

                    // We must match differently depending on the type of call we expect.
                    match call_type {
                        // If the cheatcode was called with a `count` argument,
                        // we must check that the EVM performed a CALL with this calldata exactly
                        // `count` times.
                        ExpectedCallType::Count => {
                            if *count != *actual_count {
                                let expected_values = [
                                    Some(format!("data {calldata}")),
                                    value.map(|v| format!("value {v}")),
                                    gas.map(|g| format!("gas {g}")),
                                    min_gas.map(|g| format!("minimum gas {g}")),
                                ]
                                .into_iter()
                                .flatten()
                                .join(" and ");
                                let failure_message = match status {
                                    InstructionResult::Continue | InstructionResult::Stop | InstructionResult::Return | InstructionResult::SelfDestruct =>
                                    format!("Expected call to {address:?} with {expected_values} to be called {count} time(s), but was called {actual_count} time(s)"),
                                    _ => format!("Expected call to {address:?} with {expected_values} to be called {count} time(s), but the call reverted instead. Ensure you're testing the happy path when using the expectCall cheatcode"),
                                };
                                return (
                                    InstructionResult::Revert,
                                    remaining_gas,
                                    failure_message.encode().into(),
                                )
                            }
                        }
                        // If the cheatcode was called without a `count` argument,
                        // we must check that the EVM performed a CALL with this calldata at least
                        // `count` times. The amount of times to check was
                        // the amount of time the cheatcode was called.
                        ExpectedCallType::NonCount => {
                            if *count > *actual_count {
                                let expected_values = [
                                    Some(format!("data {calldata}")),
                                    value.map(|v| format!("value {v}")),
                                    gas.map(|g| format!("gas {g}")),
                                    min_gas.map(|g| format!("minimum gas {g}")),
                                ]
                                .into_iter()
                                .flatten()
                                .join(" and ");
                                let failure_message = match status {
                                    InstructionResult::Continue | InstructionResult::Stop | InstructionResult::Return | InstructionResult::SelfDestruct =>
                                    format!("Expected call to {address:?} with {expected_values} to be called {count} time(s), but was called {actual_count} time(s)"),
                                    _ => format!("Expected call to {address:?} with {expected_values} to be called {count} time(s), but the call reverted instead. Ensure you're testing the happy path when using the expectCall cheatcode"),
                                };
                                return (
                                    InstructionResult::Revert,
                                    remaining_gas,
                                    failure_message.encode().into(),
                                )
                            }
                        }
                    }
                }
            }

            // Check if we have any leftover expected emits
            // First, if any emits were found at the root call, then we its ok and we remove them.
            self.expected_emits.retain(|expected| !expected.found);
            // If not empty, we got mismatched emits
            if !self.expected_emits.is_empty() {
                let failure_message = match status {
                    InstructionResult::Continue | InstructionResult::Stop | InstructionResult::Return | InstructionResult::SelfDestruct =>
                    "Expected an emit, but no logs were emitted afterward. You might have mismatched events or not enough events were emitted.",
                    _ => "Expected an emit, but the call reverted instead. Ensure you're testing the happy path when using the `expectEmit` cheatcode.",
                };
                return (
                    InstructionResult::Revert,
                    remaining_gas,
                    failure_message.to_string().encode().into(),
                )
            }
        }

        // if there's a revert and a previous call was diagnosed as fork related revert then we can
        // return a better error here
        if status == InstructionResult::Revert {
            if let Some(err) = self.fork_revert_diagnostic.take() {
                return (status, remaining_gas, err.to_error_msg(self).encode().into())
            }
        }

        // this will ensure we don't have false positives when trying to diagnose reverts in fork
        // mode
        let _ = self.fork_revert_diagnostic.take();

        // try to diagnose reverts in multi-fork mode where a call is made to an address that does
        // not exist
        if let TransactTo::Call(test_contract) = data.env.tx.transact_to {
            // if a call to a different contract than the original test contract returned with
            // `Stop` we check if the contract actually exists on the active fork
            if data.db.is_forked_mode() &&
                status == InstructionResult::Stop &&
                call.contract != test_contract
            {
                self.fork_revert_diagnostic =
                    data.db.diagnose_revert(b160_to_h160(call.contract), &data.journaled_state);
            }
        }

        (status, remaining_gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (InstructionResult, Option<B160>, Gas, bytes::Bytes) {
        // allow cheatcodes from the address of the new contract
        self.allow_cheatcodes_on_create(data, call);

        // Apply our prank
        if let Some(prank) = &self.prank.last() {
            if data.journaled_state.depth() >= prank.depth &&
                call.caller == h160_to_b160(prank.prank_caller)
            {
                // At the target depth we set `msg.sender`
                if data.journaled_state.depth() == prank.depth {
                    call.caller = h160_to_b160(prank.new_caller);
                }

                // At the target depth, or deeper, we set `tx.origin`
                if let Some(new_origin) = prank.new_origin {
                    data.env.tx.caller = h160_to_b160(new_origin);
                }
            }
        }

        // Apply our broadcast
        if let Some(broadcast) = &self.broadcast {
            if data.journaled_state.depth() >= broadcast.depth &&
                call.caller == h160_to_b160(broadcast.original_caller)
            {
                if let Err(err) =
                    data.journaled_state.load_account(h160_to_b160(broadcast.new_origin), data.db)
                {
                    return (
                        InstructionResult::Revert,
                        None,
                        Gas::new(call.gas_limit),
                        err.encode_string().0,
                    )
                }

                data.env.tx.caller = h160_to_b160(broadcast.new_origin);

                if data.journaled_state.depth() == broadcast.depth {
                    let (bytecode, to, nonce) = match process_create(
                        broadcast.new_origin,
                        call.init_code.clone(),
                        data,
                        call,
                    ) {
                        Ok(val) => val,
                        Err(err) => {
                            return (
                                InstructionResult::Revert,
                                None,
                                Gas::new(call.gas_limit),
                                err.encode_string().0,
                            )
                        }
                    };

                    let is_fixed_gas_limit = check_if_fixed_gas_limit(data, call.gas_limit);

                    self.broadcastable_transactions.push_back(BroadcastableTransaction {
                        rpc: data.db.active_fork_url(),
                        transaction: TypedTransaction::Legacy(TransactionRequest {
                            from: Some(broadcast.new_origin),
                            to,
                            value: Some(call.value.into()),
                            data: Some(bytecode.into()),
                            nonce: Some(nonce.into()),
                            gas: if is_fixed_gas_limit {
                                Some(call.gas_limit.into())
                            } else {
                                None
                            },
                            ..Default::default()
                        }),
                    });
                }
            }
        }

        (InstructionResult::Continue, None, Gas::new(call.gas_limit), bytes::Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: InstructionResult,
        address: Option<B160>,
        remaining_gas: Gas,
        retdata: bytes::Bytes,
    ) -> (InstructionResult, Option<B160>, Gas, bytes::Bytes) {
        // Clean up pranks
        if let Some(prank) = self.prank.last() {
            if data.journaled_state.depth() == prank.depth {
                data.env.tx.caller = h160_to_b160(prank.prank_origin);
            }
            if prank.single_call {
                let _ = self.prank.pop();
            }
        }

        // Clean up broadcasts
        if let Some(broadcast) = &self.broadcast {
            if data.journaled_state.depth() == broadcast.depth {
                data.env.tx.caller = h160_to_b160(broadcast.original_origin);
            }

            if broadcast.single_call {
                std::mem::take(&mut self.broadcast);
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if data.journaled_state.depth() <= expected_revert.depth {
                let expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                return match handle_expect_revert(
                    true,
                    expected_revert.reason.as_ref(),
                    status,
                    retdata.into(),
                ) {
                    Ok((address, retdata)) => (
                        InstructionResult::Return,
                        address.map(h160_to_b160),
                        remaining_gas,
                        retdata.0,
                    ),
                    Err(err) => {
                        (InstructionResult::Revert, None, remaining_gas, err.encode_error().0)
                    }
                }
            }
        }

        (status, address, remaining_gas, retdata)
    }
}

/// Contains additional, test specific resources that should be kept for the duration of the test
#[derive(Debug, Default)]
pub struct Context {
    //// Buffered readers for files opened for reading (path => BufReader mapping)
    pub opened_read_files: HashMap<PathBuf, BufReader<File>>,
}

/// Every time we clone `Context`, we want it to be empty
impl Clone for Context {
    fn clone(&self) -> Self {
        Default::default()
    }
}

/// Records `deal` cheatcodes
#[derive(Debug, Clone)]
pub struct DealRecord {
    /// Target of the deal.
    pub address: Address,
    /// The balance of the address before deal was applied
    pub old_balance: U256,
    /// Balance after deal was applied
    pub new_balance: U256,
}

/// Helper module to store revert strings in memory
mod revert_helper {
    use super::*;

    /// Helper that expands memory, stores a revert string pertaining to a disallowed memory write,
    /// and sets the return range to the revert string's location in memory.
    pub fn disallowed_mem_write(
        dest_offset: u64,
        size: u64,
        interpreter: &mut Interpreter,
        ranges: &[Range<u64>],
    ) {
        let revert_string: Bytes = format!(
            "Memory write at offset 0x{:02X} of size 0x{:02X} not allowed. Safe range: {}",
            dest_offset,
            size,
            ranges.iter().map(|r| format!("(0x{:02X}, 0x{:02X}]", r.start, r.end)).join(" ∪ ")
        )
        .encode()
        .into();
        mstore_revert_string(revert_string, interpreter);
    }

    /// Expands memory, stores a revert string, and sets the return range to the revert
    /// string's location in memory.
    fn mstore_revert_string(bytes: Bytes, interpreter: &mut Interpreter) {
        let starting_offset = interpreter.memory.len();
        interpreter.memory.resize(starting_offset + bytes.len());
        interpreter.memory.set_data(starting_offset, 0, bytes.len(), &bytes);
        interpreter.return_range = starting_offset..interpreter.memory.len();
    }
}
