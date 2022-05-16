/// Cheatcodes related to the execution environment.
mod env;
pub use env::{Prank, RecordAccess};
/// Assertion helpers (such as `expectEmit`)
mod expect;
pub use expect::{ExpectedCallData, ExpectedEmit, ExpectedRevert, MockCallDataContext};
/// Cheatcodes that interact with the external environment (FFI etc.)
mod ext;
/// Cheatcodes that configure the fuzzer
mod fuzz;
/// Utility cheatcodes (`sign` etc.)
mod util;

use self::expect::{handle_expect_emit, handle_expect_revert};
use crate::{
    abi::HEVMCalls,
    executor::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, AbiEncode, RawLog},
    types::{Address, H256, U256},
};
use revm::{
    opcode, BlockEnv, CallInputs, CreateInputs, Database, EVMData, Gas, Inspector, Interpreter,
    Return,
};
use std::collections::BTreeMap;

/// An inspector that handles calls to various cheatcodes, each with their own behavior.
///
/// Cheatcodes can be called by contracts during execution to modify the VM environment, such as
/// mocking addresses, signatures and altering call reverts.
#[derive(Clone, Debug, Default)]
pub struct Cheatcodes {
    /// Whether FFI is enabled or not
    pub ffi: bool,

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

    /// Recorded storage reads and writes
    pub accesses: Option<RecordAccess>,

    /// Mocked calls
    pub mocked_calls: BTreeMap<Address, BTreeMap<MockCallDataContext, Bytes>>,

    /// Expected calls
    pub expected_calls: BTreeMap<Address, Vec<ExpectedCallData>>,

    /// Expected emits
    pub expected_emits: Vec<ExpectedEmit>,
}

impl Cheatcodes {
    pub fn new(ffi: bool, block: BlockEnv, gas_price: U256) -> Self {
        Self { ffi, block: Some(block), gas_price: Some(gas_price), ..Default::default() }
    }

    fn apply_cheatcode<DB: Database>(
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
            .or_else(|| ext::apply(self.ffi, &decoded))
            .ok_or_else(|| "Cheatcode was unhandled. This is a bug.".to_string().encode())?
    }
}

impl<DB> Inspector<DB> for Cheatcodes
where
    DB: Database,
{
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
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
                    *mock.calldata == call.input[..mock.calldata.len()] &&
                        mock.value.map(|value| value == call.transfer.value).unwrap_or(true)
                }) {
                    return (Return::Return, Gas::new(call.gas_limit), mock_retdata.clone())
                }
            }

            // Apply our prank
            if let Some(prank) = &self.prank {
                if data.subroutine.depth() >= prank.depth &&
                    call.context.caller == prank.prank_caller
                {
                    // At the target depth we set `msg.sender`
                    if data.subroutine.depth() == prank.depth {
                        call.context.caller = prank.new_caller;
                        call.transfer.source = prank.new_caller;
                    }

                    // At the target depth, or deeper, we set `tx.origin`
                    if let Some(new_origin) = prank.new_origin {
                        data.env.tx.caller = new_origin;
                    }
                }
            }

            (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
        } else {
            (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
        }
    }

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
            match interpreter.contract.code[interpreter.program_counter()] {
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
            if data.subroutine.depth() == prank.depth {
                data.env.tx.caller = prank.prank_origin;
            }
            if prank.single_call {
                std::mem::take(&mut self.prank);
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if data.subroutine.depth() <= expected_revert.depth {
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
            .filter(|expected| expected.depth == data.subroutine.depth())
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
        if data.subroutine.depth() == 0 {
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

        (status, remaining_gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        // Apply our prank
        if let Some(prank) = &self.prank {
            if data.subroutine.depth() >= prank.depth && call.caller == prank.prank_caller {
                // At the target depth we set `msg.sender`
                if data.subroutine.depth() == prank.depth {
                    call.caller = prank.new_caller;
                }

                // At the target depth, or deeper, we set `tx.origin`
                if let Some(new_origin) = prank.new_origin {
                    data.env.tx.caller = new_origin;
                }
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
            if data.subroutine.depth() == prank.depth {
                data.env.tx.caller = prank.prank_origin;
            }
            if prank.single_call {
                std::mem::take(&mut self.prank);
            }
        }

        // Handle expected reverts
        if let Some(expected_revert) = &self.expected_revert {
            if data.subroutine.depth() <= expected_revert.depth {
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
