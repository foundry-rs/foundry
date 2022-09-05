use self::{
    env::Broadcast,
    expect::{handle_expect_emit, handle_expect_revert},
    util::process_create,
};
use crate::{
    abi::HEVMCalls,
    executor::{
        backend::DatabaseExt, inspector::cheatcodes::env::RecordedLogs, CHEATCODE_ADDRESS,
        HARDHAT_CONSOLE_ADDRESS,
    },
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, AbiEncode, RawLog},
    types::{
        transaction::eip2718::TypedTransaction, Address, NameOrAddress, TransactionRequest, H256,
        U256,
    },
};
use revm::{
    opcode, BlockEnv, CallInputs, CreateInputs, EVMData, Gas, Inspector, Interpreter, Return,
    TransactTo,
};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs::File,
    io::BufReader,
    path::PathBuf,
    sync::Arc,
};

/// Cheatcodes related to the execution environment.
mod env;
pub use env::{Prank, RecordAccess};
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
pub use util::{DEFAULT_CREATE2_DEPLOYER, MISSING_CREATE2_DEPLOYER};

mod config;
use crate::executor::backend::RevertDiagnostic;
pub use config::CheatsConfig;

/// An inspector that handles calls to various cheatcodes, each with their own behavior.
///
/// Cheatcodes can be called by contracts during execution to modify the VM environment, such as
/// mocking addresses, signatures and altering call reverts.
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

    /// Current broadcasting information
    pub broadcast: Option<Broadcast>,

    /// Used to correct the nonce of --sender after the initiating call
    pub corrected_nonce: bool,

    /// Scripting based transactions
    pub broadcastable_transactions: VecDeque<TypedTransaction>,

    /// Additional, user configurable context this Inspector has access to when inspecting a call
    pub config: Arc<CheatsConfig>,

    /// Test-scoped context holding data that needs to be reset every test run
    pub context: Context,

    // Commit FS changes such as file creations, writes and deletes.
    // Used to prevent duplicate changes file executing non-committing calls.
    pub fs_commit: bool,
}

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

impl Cheatcodes {
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

        // TODO: Log the opcode for the debugger
        env::apply(self, data, caller, &decoded)
            .or_else(|| util::apply(self, data, &decoded))
            .or_else(|| expect::apply(self, data, &decoded))
            .or_else(|| fuzz::apply(data, &decoded))
            .or_else(|| ext::apply(self, self.config.ffi, &decoded))
            .or_else(|| snapshot::apply(self, data, &decoded))
            .or_else(|| fork::apply(self, data, &decoded))
            .ok_or_else(|| "Cheatcode was unhandled. This is a bug.".to_string().encode())?
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

    fn step(&mut self, interpreter: &mut Interpreter, _: &mut EVMData<'_, DB>, _: bool) -> Return {
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
            storage_recorded_logs
                .entries
                .push(RawLog { topics: topics.to_vec(), data: data.to_vec() });
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
                        expected.value.map(|value| value == call.transfer.value).unwrap_or(true)
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
                    call.context.caller = broadcast.origin;
                    call.transfer.source = broadcast.origin;
                    // Add a `legacy` transaction to the VecDeque. We use a legacy transaction here
                    // because we only need the from, to, value, and data. We can later change this
                    // into 1559, in the cli package, relatively easily once we
                    // know the target chain supports EIP-1559.
                    if !is_static {
                        data.journaled_state.load_account(broadcast.origin, data.db);
                        let account =
                            data.journaled_state.state().get_mut(&broadcast.origin).unwrap();

                        self.broadcastable_transactions.push_back(TypedTransaction::Legacy(
                            TransactionRequest {
                                from: Some(broadcast.origin),
                                to: Some(NameOrAddress::Address(call.contract)),
                                value: Some(call.transfer.value),
                                data: Some(call.input.clone().into()),
                                nonce: Some(account.info.nonce.into()),
                                ..Default::default()
                            },
                        ));

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
            if broadcast.single_call {
                std::mem::take(&mut self.broadcast);
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if data.journaled_state.depth() <= expected_revert.depth {
                let expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                return match handle_expect_revert(false, &expected_revert.reason, status, retdata) {
                    Err(retdata) => (Return::Revert, remaining_gas, retdata),
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
                return (
                    Return::Revert,
                    remaining_gas,
                    format!(
                        "Expected a call to {:?} with data {}{}, but got none",
                        address,
                        ethers::types::Bytes::from(expecteds[0].calldata.clone()),
                        expecteds[0].value.map(|v| format!(" and value {}", v)).unwrap_or_default()
                    )
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
                data.journaled_state.load_account(broadcast.origin, data.db);

                let (bytecode, to, nonce) =
                    process_create(broadcast.origin, call.init_code.clone(), data, call);

                self.broadcastable_transactions.push_back(TypedTransaction::Legacy(
                    TransactionRequest {
                        from: Some(broadcast.origin),
                        to,
                        value: Some(call.value),
                        data: Some(bytecode.into()),
                        nonce: Some(nonce.into()),
                        ..Default::default()
                    },
                ));
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
            if broadcast.single_call {
                std::mem::take(&mut self.broadcast);
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if data.journaled_state.depth() <= expected_revert.depth {
                let expected_revert = std::mem::take(&mut self.expected_revert).unwrap();
                return match handle_expect_revert(true, &expected_revert.reason, status, retdata) {
                    Err(retdata) => (Return::Revert, None, remaining_gas, retdata),
                    Ok((address, retdata)) => (Return::Return, address, remaining_gas, retdata),
                }
            }
        }

        (status, address, remaining_gas, retdata)
    }
}
