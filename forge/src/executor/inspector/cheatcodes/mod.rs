/// Cheatcodes related to the execution environment.
mod env;
pub use env::{Prank, RecordAccess};
/// Assertion helpers (such as `expectEmit`)
mod expect;
pub use expect::ExpectedRevert;
/// Cheatcodes that interact with the external environment (FFI etc.)
mod ext;
/// Cheatcodes that configure the fuzzer
mod fuzz;
/// Utility cheatcodes (`sign` etc.)
mod util;

use crate::{abi::HEVMCalls, executor::CHEATCODE_ADDRESS};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, AbiEncode},
    types::Address,
};
use revm::{
    opcode, CallInputs, CreateInputs, Database, EVMData, Gas, Inspector, Interpreter, Return,
};
use std::collections::BTreeMap;

use self::expect::handle_expect_revert;

/// An inspector that handles calls to various cheatcodes, each with their own behavior.
///
/// Cheatcodes can be called by contracts during execution to modify the VM environment, such as
/// mocking addresses, signatures and altering call reverts.
pub struct Cheatcodes {
    /// Whether FFI is enabled or not
    ffi: bool,

    /// Address labels
    pub labels: BTreeMap<Address, String>,

    /// Prank information
    pub prank: Option<Prank>,

    /// Expected revert information
    pub expected_revert: Option<ExpectedRevert>,

    /// Recorded storage reads and writes
    pub accesses: Option<RecordAccess>,

    /// Mocked calls
    pub mocked_calls: BTreeMap<Address, BTreeMap<Bytes, Bytes>>,
}

impl Cheatcodes {
    pub fn new(ffi: bool) -> Self {
        Self {
            ffi,
            labels: BTreeMap::new(),
            prank: None,
            expected_revert: None,
            accesses: None,
            mocked_calls: BTreeMap::new(),
        }
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
        call: &CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == *CHEATCODE_ADDRESS {
            match self.apply_cheatcode(data, call.context.caller, call) {
                Ok(retdata) => (Return::Return, Gas::new(0), retdata),
                Err(err) => (Return::Revert, Gas::new(0), err),
            }
        } else {
            // Handle mocked calls
            if let Some(mocks) = self.mocked_calls.get(&call.contract) {
                if let Some(mock_retdata) = mocks.get(&call.input) {
                    return (Return::Return, Gas::new(0), mock_retdata.clone())
                } else if let Some((_, mock_retdata)) =
                    mocks.iter().find(|(mock, _)| *mock == &call.input[..mock.len()])
                {
                    return (Return::Return, Gas::new(0), mock_retdata.clone())
                }
            }

            (Return::Continue, Gas::new(0), Bytes::new())
        }
    }

    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _: bool,
    ) -> Return {
        // Apply our prank
        if let Some(prank) = &self.prank {
            if data.subroutine.depth() >= prank.depth &&
                interpreter.contract.caller == prank.prank_caller
            {
                // At the target depth we set `msg.sender`
                if data.subroutine.depth() == prank.depth {
                    interpreter.contract.caller = prank.new_caller;
                }

                // At the target depth, or deeper, we set `tx.origin`
                if let Some(new_origin) = prank.new_origin {
                    data.env.tx.caller = new_origin;
                }
            }
        }

        Return::Continue
    }

    fn step(&mut self, interpreter: &mut Interpreter, _: &mut EVMData<'_, DB>, _: bool) -> Return {
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

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: Return,
        retdata: Bytes,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == *CHEATCODE_ADDRESS {
            return (status, remaining_gas, retdata)
        }

        // Clean up pranks
        if let Some(prank) = &self.prank {
            data.env.tx.caller = prank.prank_origin;
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

        (status, remaining_gas, retdata)
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
            data.env.tx.caller = prank.prank_origin;
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
