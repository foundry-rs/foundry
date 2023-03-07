use self::{
    env::Broadcast,
    expect::{handle_expect_emit, handle_expect_revert},
    util::{check_if_fixed_gas_limit, process_create, BroadcastableTransactions},
};
use crate::{
    abi::HEVMCalls,
    error::SolError,
    executor::{
        backend::DatabaseExt, inspector::cheatcodes::env::RecordedLogs, CHEATCODE_ADDRESS,
        HARDHAT_CONSOLE_ADDRESS,
    },
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, AbiEncode, RawLog},
    signers::LocalWallet,
    types::{
        transaction::eip2718::TypedTransaction, Address, NameOrAddress, TransactionRequest, H256,
        U256,
    },
};
use itertools::Itertools;
use revm::{
    opcode, BlockEnv, CallInputs, CreateInputs, EVMData, Gas, Inspector, Interpreter, Return,
    TransactTo,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::BufReader,
    ops::Range,
    path::PathBuf,
    sync::Arc,
};
use tracing::trace;

/// Cheatcodes related to the execution environment.
mod env;
pub use env::{Log, Prank, RecordAccess};
/// Assertion helpers (such as `expectEmit`)
mod expect;
pub use expect::{ExpectedCallData, ExpectedEmit, ExpectedRevert, MockCallDataContext};

/// Cheatcodes that interact with the external environment (FFI etc.)
mod ext;
/// Fork related cheatcodes
mod fork;
/// Cheatcodes that configure the fuzzer
mod fuzz;
/// Snapshot related cheatcodes
mod snapshot;
/// Utility cheatcodes (`sign` etc.)
pub mod util;
pub use util::{BroadcastableTransaction, DEFAULT_CREATE2_DEPLOYER};

mod config;
use crate::executor::{backend::RevertDiagnostic, inspector::utils::get_create_address};
pub use config::CheatsConfig;

mod error;

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

    /// Prank information
    pub prank: Option<Prank>,

    /// Expected revert information
    pub expected_revert: Option<ExpectedRevert>,

    /// Additional diagnostic for reverts
    pub fork_revert_diagnostic: Option<RevertDiagnostic>,

    /// Recorded storage reads and writes
    pub accesses: Option<RecordAccess>,

    /// Recorded logs
    pub recorded_logs: Option<RecordedLogs>,

    /// Mocked calls
    pub mocked_calls: BTreeMap<Address, BTreeMap<MockCallDataContext, Bytes>>,

    /// Expected calls
    pub expected_calls: BTreeMap<Address, Vec<ExpectedCallData>>,

    /// Expected emits
    pub expected_emits: Vec<ExpectedEmit>,

    /// Map of context depths to memory offset ranges that may be written to within the call depth.
    pub allowed_mem_writes: HashMap<u64, Vec<Range<u64>>>,

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

    pub serialized_jsons: HashMap<String, HashMap<String, Value>>,

    /// Records all eth deals
    pub eth_deals: Vec<DealRecord>,

    /// Holds the stored gas info for when we pause gas metering. It is an `Option<Option<..>>`
    /// because the `call` callback in an `Inspector` doesn't get access to
    /// the `revm::Interpreter` which holds the `revm::Gas` struct that
    /// we need to copy. So we convert it to a `Some(None)` in `apply_cheatcode`, and once we have
    /// the interpreter, we copy the gas struct. Then each time there is an execution of an
    /// operation, we reset the gas.
    pub gas_metering: Option<Option<revm::Gas>>,

    /// Holds stored gas info for when we pause gas metering, and we're entering/inside
    /// CREATE / CREATE2 frames. This is needed to make gas meter pausing work correctly when
    /// paused and creating new contracts.
    pub gas_metering_create: Option<Option<revm::Gas>>,
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

    #[tracing::instrument(skip_all, name = "applying cheatcode")]
    fn apply_cheatcode<DB: DatabaseExt>(
        &mut self,
        data: &mut EVMData<'_, DB>,
        caller: Address,
        call: &CallInputs,
    ) -> Result<Bytes, Bytes> {
        // Decode the cheatcode call
        let decoded = HEVMCalls::decode(&call.input).map_err(|err| err.to_string().encode())?;

        // ensure the caller is allowed to execute cheatcodes, but only if the backend is in forking
        // mode
        data.db.ensure_cheatcode_access_forking_mode(caller).map_err(|err| err.encode_string())?;

        // TODO: Log the opcode for the debugger
        env::apply(self, data, caller, &decoded)
            .transpose()
            .or_else(|| util::apply(self, data, &decoded))
            .or_else(|| expect::apply(self, data, &decoded))
            .or_else(|| fuzz::apply(data, &decoded))
            .or_else(|| ext::apply(self, self.config.ffi, &decoded))
            .or_else(|| snapshot::apply(self, data, &decoded))
            .or_else(|| fork::apply(self, data, &decoded))
            .ok_or_else(|| "Cheatcode was unhandled. This is a bug.".to_string().encode())?
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

        if data.journaled_state.depth > 1 && !data.db.has_cheatcode_access(inputs.caller) {
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
            if let Some(acc) = data.journaled_state.state.get_mut(&record.address) {
                acc.info.balance = record.old_balance;
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
    ) -> Return {
        // When the first interpreter is initialized we've circumvented the balance and gas checks,
        // so we apply our actual block data with the correct fees and all.
        if let Some(block) = self.block.take() {
            data.env.block = block;
        }
        if let Some(gas_price) = self.gas_price.take() {
            data.env.tx.gas_price = gas_price;
        }

        Return::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _: bool,
    ) -> Return {
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
                                interpreter.gas = revm::Gas::new(0);
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
                                interpreter.gas = revm::Gas::new(gas.limit());

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
                        .entry(interpreter.contract().address)
                        .or_insert_with(Vec::new)
                        .push(key);
                }
                opcode::SSTORE => {
                    let key = try_or_continue!(interpreter.stack().peek(0));

                    // An SSTORE does an SLOAD internally
                    storage_accesses
                        .reads
                        .entry(interpreter.contract().address)
                        .or_insert_with(Vec::new)
                        .push(key);
                    storage_accesses
                        .writes
                        .entry(interpreter.contract().address)
                        .or_insert_with(Vec::new)
                        .push(key);
                }
                _ => (),
            }
        }

        // If the allowed memory writes cheatcode is active at this context depth, check to see
        // if the current opcode is either an `MSTORE` or `MSTORE8`. If the opcode at the current
        // program counter is a match, check if the offset passed to the opcode lies within the
        // allowed ranges. If not, revert and fail the test.
        if let Some(ranges) = self.allowed_mem_writes.get(&data.journaled_state.depth()) {
            macro_rules! mem_opcode_match {
                ([$(($opcode:ident, $offset_depth:expr, $size_depth:expr)),*]) => {
                    match interpreter.contract.bytecode.bytecode()[interpreter.program_counter()] {
                        opcode::MSTORE => {
                            // The offset of the mstore operation is at the top of the stack.
                            let offset = try_or_continue!(interpreter.stack().peek(0)).as_u64();

                            // If none of the allowed ranges contain [offset, offset + 32), memory has been
                            // unexpectedly mutated.
                            if !ranges
                                .iter()
                                    .any(|range| range.contains(&offset) && range.contains(&(offset + 31)))
                                    {
                                        // TODO: Revert message
                                        return Return::Revert
                                    }
                        }
                        opcode::MSTORE8 => {
                            // The offset of the mstore8 operation is at the top of the stack.
                            let offset = try_or_continue!(interpreter.stack().peek(0)).as_u64();

                            // If none of the allowed ranges contain the offset, memory has been
                            // unexpectedly mutated.
                            if !ranges.iter().any(|range| range.contains(&offset)) {
                                // TODO: Revert message
                                return Return::Revert
                            }
                        }
                        $(opcode::$opcode => {
                            // The destination offset of the operation is at the top of the stack.
                            let dest_offset = try_or_continue!(interpreter.stack().peek($offset_depth)).as_u64();

                            // The size of the data that will be copied is the third item on the stack.
                            let size = try_or_continue!(interpreter.stack().peek($size_depth)).as_u64();

                            // If none of the allowed ranges contain [dest_offset, dest_offset + size),
                            // memory has been unexpectedly mutated.
                            if !ranges.iter().any(|range| {
                                range.contains(&dest_offset) &&
                                    range.contains(&(dest_offset + size.saturating_sub(1)))
                            }) {
                                return Return::Revert
                            }
                        })*
                        _ => ()
                    }
                }
            }

            // Check if the current opcode can write to memory, and if so, check if the memory
            // being written to is registered as safe to modify.
            mem_opcode_match!([
                (CALLDATACOPY, 0, 2),
                (CODECOPY, 0, 2),
                (RETURNDATACOPY, 0, 2),
                (EXTCODECOPY, 1, 3),
                (CALL, 5, 6),
                (CALLCODE, 5, 6),
                (STATICCALL, 4, 5),
                (DELEGATECALL, 4, 5)
            ])
        }

        Return::Continue
    }

    fn log(&mut self, _: &mut EVMData<'_, DB>, address: &Address, topics: &[H256], data: &Bytes) {
        // Match logs if `expectEmit` has been called
        if !self.expected_emits.is_empty() {
            handle_expect_emit(
                self,
                RawLog { topics: topics.to_vec(), data: data.to_vec() },
                address,
            );
        }

        // Stores this log if `recordLogs` has been called
        if let Some(storage_recorded_logs) = &mut self.recorded_logs {
            storage_recorded_logs.entries.push(Log {
                emitter: *address,
                inner: RawLog { topics: topics.to_vec(), data: data.to_vec() },
            });
        }
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == CHEATCODE_ADDRESS {
            match self.apply_cheatcode(data, call.context.caller, call) {
                Ok(retdata) => (Return::Return, Gas::new(call.gas_limit), retdata),
                Err(err) => (Return::Revert, Gas::new(call.gas_limit), err),
            }
        } else if call.contract != HARDHAT_CONSOLE_ADDRESS {
            // Handle expected calls
            if let Some(expecteds) = self.expected_calls.get_mut(&call.contract) {
                if let Some(found_match) = expecteds.iter().position(|expected| {
                    expected.calldata.len() <= call.input.len() &&
                        expected.calldata == call.input[..expected.calldata.len()] &&
                        expected.value.map_or(true, |value| value == call.transfer.value) &&
                        expected.gas.map_or(true, |gas| gas == call.gas_limit) &&
                        expected.min_gas.map_or(true, |min_gas| min_gas <= call.gas_limit)
                }) {
                    expecteds.remove(found_match);
                }
            }

            // Handle mocked calls
            if let Some(mocks) = self.mocked_calls.get(&call.contract) {
                let ctx = MockCallDataContext {
                    calldata: call.input.clone(),
                    value: Some(call.transfer.value),
                };
                if let Some(mock_retdata) = mocks.get(&ctx) {
                    return (Return::Return, Gas::new(call.gas_limit), mock_retdata.clone())
                } else if let Some((_, mock_retdata)) = mocks.iter().find(|(mock, _)| {
                    mock.calldata.len() <= call.input.len() &&
                        *mock.calldata == call.input[..mock.calldata.len()] &&
                        mock.value.map(|value| value == call.transfer.value).unwrap_or(true)
                }) {
                    return (Return::Return, Gas::new(call.gas_limit), mock_retdata.clone())
                }
            }

            // Apply our prank
            if let Some(prank) = &self.prank {
                if data.journaled_state.depth() >= prank.depth &&
                    call.context.caller == prank.prank_caller
                {
                    // At the target depth we set `msg.sender`
                    if data.journaled_state.depth() == prank.depth {
                        call.context.caller = prank.new_caller;
                        call.transfer.source = prank.new_caller;
                    }

                    // At the target depth, or deeper, we set `tx.origin`
                    if let Some(new_origin) = prank.new_origin {
                        data.env.tx.caller = new_origin;
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
                    call.context.caller == broadcast.original_caller
                {
                    // At the target depth we set `msg.sender` & tx.origin.
                    // We are simulating the caller as being an EOA, so *both* must be set to the
                    // broadcast.origin.
                    data.env.tx.caller = broadcast.new_origin;

                    call.context.caller = broadcast.new_origin;
                    call.transfer.source = broadcast.new_origin;
                    // Add a `legacy` transaction to the VecDeque. We use a legacy transaction here
                    // because we only need the from, to, value, and data. We can later change this
                    // into 1559, in the cli package, relatively easily once we
                    // know the target chain supports EIP-1559.
                    if !is_static {
                        if let Err(err) =
                            data.journaled_state.load_account(broadcast.new_origin, data.db)
                        {
                            return (Return::Revert, Gas::new(call.gas_limit), err.encode_string())
                        }

                        let is_fixed_gas_limit = check_if_fixed_gas_limit(data, call.gas_limit);

                        let account =
                            data.journaled_state.state().get_mut(&broadcast.new_origin).unwrap();

                        self.broadcastable_transactions.push_back(BroadcastableTransaction {
                            rpc: data.db.active_fork_url(),
                            transaction: TypedTransaction::Legacy(TransactionRequest {
                                from: Some(broadcast.new_origin),
                                to: Some(NameOrAddress::Address(call.contract)),
                                value: Some(call.transfer.value),
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
                            Return::Revert,
                            Gas::new(0),
                            "Staticcalls are not allowed after vm.broadcast. Either remove it, or use vm.startBroadcast instead."
                            .to_string()
                            .encode()
                            .into()
                        );
                    }
                }
            }

            (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
        } else {
            (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
        }
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == CHEATCODE_ADDRESS || call.contract == HARDHAT_CONSOLE_ADDRESS {
            return (status, remaining_gas, retdata)
        }

        // Clean up pranks
        if let Some(prank) = &self.prank {
            if data.journaled_state.depth() == prank.depth {
                data.env.tx.caller = prank.prank_origin;
            }
            if prank.single_call {
                std::mem::take(&mut self.prank);
            }
        }

        // Clean up broadcast
        if let Some(broadcast) = &self.broadcast {
            if data.journaled_state.depth() == broadcast.depth {
                data.env.tx.caller = broadcast.original_origin;
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
                    retdata,
                ) {
                    Err(retdata) => {
                        trace!(expected=?expected_revert, actual=%hex::encode(&retdata), ?status, "Expected revert mismatch");
                        (Return::Revert, remaining_gas, retdata)
                    }
                    Ok((_, retdata)) => (Return::Return, remaining_gas, retdata),
                }
            }
        }

        // Handle expected emits at current depth
        if !self
            .expected_emits
            .iter()
            .filter(|expected| expected.depth == data.journaled_state.depth())
            .all(|expected| expected.found)
        {
            return (
                Return::Revert,
                remaining_gas,
                "Log != expected log".to_string().encode().into(),
            )
        } else {
            // Clear the emits we expected at this depth that have been found
            self.expected_emits.retain(|expected| !expected.found)
        }

        // If the depth is 0, then this is the root call terminating
        if data.journaled_state.depth() == 0 {
            // Handle expected calls that were not fulfilled
            if let Some((address, expecteds)) =
                self.expected_calls.iter().find(|(_, expecteds)| !expecteds.is_empty())
            {
                let ExpectedCallData { calldata, gas, min_gas, value } = &expecteds[0];
                let calldata = ethers::types::Bytes::from(calldata.clone());
                let expected_values = [
                    Some(format!("data {calldata}")),
                    value.map(|v| format!("value {v}")),
                    gas.map(|g| format!("gas {g}")),
                    min_gas.map(|g| format!("minimum gas {g}")),
                ]
                .into_iter()
                .flatten()
                .join(" and ");
                return (
                    Return::Revert,
                    remaining_gas,
                    format!("Expected a call to {address:?} with {expected_values}, but got none")
                        .encode()
                        .into(),
                )
            }

            // Check if we have any leftover expected emits
            if !self.expected_emits.is_empty() {
                return (
                    Return::Revert,
                    remaining_gas,
                    "Expected an emit, but no logs were emitted afterward"
                        .to_string()
                        .encode()
                        .into(),
                )
            }
        }

        // if there's a revert and a previous call was diagnosed as fork related revert then we can
        // return a better error here
        if status == Return::Revert {
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
            if data.db.is_forked_mode() && status == Return::Stop && call.contract != test_contract
            {
                self.fork_revert_diagnostic =
                    data.db.diagnose_revert(call.contract, &data.journaled_state);
            }
        }

        (status, remaining_gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        // allow cheatcodes from the address of the new contract
        self.allow_cheatcodes_on_create(data, call);

        // Apply our prank
        if let Some(prank) = &self.prank {
            if data.journaled_state.depth() >= prank.depth && call.caller == prank.prank_caller {
                // At the target depth we set `msg.sender`
                if data.journaled_state.depth() == prank.depth {
                    call.caller = prank.new_caller;
                }

                // At the target depth, or deeper, we set `tx.origin`
                if let Some(new_origin) = prank.new_origin {
                    data.env.tx.caller = new_origin;
                }
            }
        }

        // Apply our broadcast
        if let Some(broadcast) = &self.broadcast {
            if data.journaled_state.depth() == broadcast.depth &&
                call.caller == broadcast.original_caller
            {
                if let Err(err) = data.journaled_state.load_account(broadcast.new_origin, data.db) {
                    return (Return::Revert, None, Gas::new(call.gas_limit), err.encode_string())
                }

                data.env.tx.caller = broadcast.new_origin;

                let (bytecode, to, nonce) = match process_create(
                    broadcast.new_origin,
                    call.init_code.clone(),
                    data,
                    call,
                ) {
                    Ok(val) => val,
                    Err(err) => {
                        return (Return::Revert, None, Gas::new(call.gas_limit), err.encode_string())
                    }
                };

                let is_fixed_gas_limit = check_if_fixed_gas_limit(data, call.gas_limit);

                self.broadcastable_transactions.push_back(BroadcastableTransaction {
                    rpc: data.db.active_fork_url(),
                    transaction: TypedTransaction::Legacy(TransactionRequest {
                        from: Some(broadcast.new_origin),
                        to,
                        value: Some(call.value),
                        data: Some(bytecode.into()),
                        nonce: Some(nonce.into()),
                        gas: if is_fixed_gas_limit { Some(call.gas_limit.into()) } else { None },
                        ..Default::default()
                    }),
                });
            }
        }

        (Return::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        _: &CreateInputs,
        status: Return,
        address: Option<Address>,
        remaining_gas: Gas,
        retdata: Bytes,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        // Clean up pranks
        if let Some(prank) = &self.prank {
            if data.journaled_state.depth() == prank.depth {
                data.env.tx.caller = prank.prank_origin;
            }
            if prank.single_call {
                std::mem::take(&mut self.prank);
            }
        }

        // Clean up broadcasts
        if let Some(broadcast) = &self.broadcast {
            if data.journaled_state.depth() == broadcast.depth {
                data.env.tx.caller = broadcast.original_origin;
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
                    retdata,
                ) {
                    Err(retdata) => (Return::Revert, None, remaining_gas, retdata),
                    Ok((address, retdata)) => (Return::Return, address, remaining_gas, retdata),
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
