use crate::inspector::Cheatcodes;
use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::backend::DatabaseExt;
use revm::{
    interpreter::{
        CreateInputs, CreateOutcome, CreateScheme, EOFCreateInput, EOFCreateOutcome, Gas,
        InstructionResult, InterpreterResult,
    },
    InnerEvmContext,
};
use std::ops::Range;

/// Common behaviour of legacy and EOF create inputs.
pub(crate) trait CommonCreateInput<DB: DatabaseExt> {
    fn caller(&self) -> Address;
    fn gas_limit(&self) -> u64;
    fn value(&self) -> U256;
    fn init_code(&self) -> Bytes;
    fn scheme(&self) -> Option<CreateScheme>;
    fn set_caller(&mut self, caller: Address);
    fn create_outcome(
        &self,
        interpreter_result: InterpreterResult,
        created_address: Option<Address>,
        return_memory_range: Option<Range<usize>>,
    ) -> CommonCreateOutcome;
    fn log_debug(&self, cheatcode: &mut Cheatcodes, scheme: &CreateScheme);
    fn allow_cheatcodes(
        &self,
        cheatcodes: &mut Cheatcodes,
        ecx: &mut InnerEvmContext<DB>,
    ) -> Address;
    fn computed_created_address(&self) -> Option<Address>;
    fn return_memory_range(&self) -> Option<Range<usize>>;
}

impl<DB: DatabaseExt> CommonCreateInput<DB> for &mut CreateInputs {
    fn caller(&self) -> Address {
        self.caller
    }
    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
    fn value(&self) -> U256 {
        self.value
    }
    fn init_code(&self) -> Bytes {
        self.init_code.clone()
    }
    fn scheme(&self) -> Option<CreateScheme> {
        Some(self.scheme)
    }
    fn set_caller(&mut self, caller: Address) {
        self.caller = caller;
    }
    fn create_outcome(
        &self,
        result: InterpreterResult,
        address: Option<Address>,
        _return_memory_range: Option<Range<usize>>,
    ) -> CommonCreateOutcome {
        CommonCreateOutcome::Create(CreateOutcome { result, address })
    }
    fn log_debug(&self, cheatcode: &mut Cheatcodes, scheme: &CreateScheme) {
        let kind = match scheme {
            CreateScheme::Create => "create",
            CreateScheme::Create2 { .. } => "create2",
        };
        debug!(target: "cheatcodes", tx=?cheatcode.broadcastable_transactions.back().unwrap(), "broadcastable {kind}");
    }
    fn allow_cheatcodes(
        &self,
        cheatcodes: &mut Cheatcodes,
        ecx: &mut InnerEvmContext<DB>,
    ) -> Address {
        let old_nonce = ecx
            .journaled_state
            .state
            .get(&self.caller)
            .map(|acc| acc.info.nonce)
            .unwrap_or_default();
        let created_address = self.created_address(old_nonce);
        cheatcodes.allow_cheatcodes_on_create(ecx, self.caller, created_address);
        created_address
    }
    fn computed_created_address(&self) -> Option<Address> {
        None
    }
    fn return_memory_range(&self) -> Option<Range<usize>> {
        None
    }
}

impl<DB: DatabaseExt> CommonCreateInput<DB> for &mut EOFCreateInput {
    fn caller(&self) -> Address {
        self.caller
    }
    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
    fn value(&self) -> U256 {
        self.value
    }
    fn init_code(&self) -> Bytes {
        self.eof_init_code.raw.clone()
    }
    fn scheme(&self) -> Option<CreateScheme> {
        None
    }
    fn set_caller(&mut self, caller: Address) {
        self.caller = caller;
    }
    fn create_outcome(
        &self,
        result: InterpreterResult,
        address: Option<Address>,
        return_memory_range: Option<Range<usize>>,
    ) -> CommonCreateOutcome {
        CommonCreateOutcome::EOFCreate(EOFCreateOutcome {
            result,
            address: address.unwrap_or(self.created_address),
            return_memory_range: return_memory_range.unwrap_or_default(),
        })
    }
    fn log_debug(&self, cheatcode: &mut Cheatcodes, _scheme: &CreateScheme) {
        debug!(target: "cheatcodes", tx=?cheatcode.broadcastable_transactions.back().unwrap(), "broadcastable eofcreate");
    }
    fn allow_cheatcodes(
        &self,
        cheatcodes: &mut Cheatcodes,
        ecx: &mut InnerEvmContext<DB>,
    ) -> Address {
        cheatcodes.allow_cheatcodes_on_create(ecx, self.caller, self.created_address);
        self.created_address
    }
    fn computed_created_address(&self) -> Option<Address> {
        Some(self.created_address)
    }
    fn return_memory_range(&self) -> Option<Range<usize>> {
        Some(self.return_memory_range.clone())
    }
}

pub(crate) enum CommonCreateOutcome {
    Create(CreateOutcome),
    EOFCreate(EOFCreateOutcome),
}

/// Common behaviour of legacy and EOF create_end inputs.
pub(crate) trait CommonEndInput {
    fn outcome_result(&self) -> InstructionResult;
    fn outcome_output(&self) -> Bytes;
    fn outcome_gas(&self) -> Gas;
    fn address(&self) -> Option<Address>;
    fn create_outcome(
        &self,
        result: InstructionResult,
        output: Bytes,
        address: Option<Address>,
    ) -> CommonEndOutcome;
}

impl CommonEndInput for CreateOutcome {
    fn outcome_result(&self) -> InstructionResult {
        self.result.result
    }
    fn outcome_output(&self) -> Bytes {
        self.result.output.clone()
    }
    fn outcome_gas(&self) -> Gas {
        self.result.gas
    }
    fn address(&self) -> Option<Address> {
        self.address
    }

    fn create_outcome(
        &self,
        result: InstructionResult,
        output: Bytes,
        address: Option<Address>,
    ) -> CommonEndOutcome {
        CommonEndOutcome::Create(Self {
            result: InterpreterResult { result, output, gas: self.outcome_gas() },
            address,
        })
    }
}

impl CommonEndInput for EOFCreateOutcome {
    fn outcome_result(&self) -> InstructionResult {
        self.result.result
    }
    fn outcome_output(&self) -> Bytes {
        self.result.output.clone()
    }
    fn outcome_gas(&self) -> Gas {
        self.result.gas
    }
    fn address(&self) -> Option<Address> {
        Some(self.address)
    }
    fn create_outcome(
        &self,
        result: InstructionResult,
        output: Bytes,
        address: Option<Address>,
    ) -> CommonEndOutcome {
        CommonEndOutcome::EOFCreate(Self {
            result: InterpreterResult { result, output, gas: self.outcome_gas() },
            address: address.unwrap_or_default(),
            return_memory_range: self.return_memory_range.clone(),
        })
    }
}

pub(crate) enum CommonEndOutcome {
    Create(CreateOutcome),
    EOFCreate(EOFCreateOutcome),
}
