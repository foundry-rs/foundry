use super::InvariantContract;
use crate::executors::RawCallResult;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes};
use foundry_evm_core::decode::RevertDecoder;
use foundry_evm_fuzz::{BasicTxDetails, Reason, invariant::FuzzRunIdentifiedContracts};
use proptest::test_runner::TestError;
use std::{collections::HashMap, fmt};

/// Stores information about failures and reverts of the invariant tests.
#[derive(Clone, Default)]
pub struct InvariantFailures {
    /// Total number of reverts.
    pub reverts: usize,
    /// The latest revert reason of a run.
    pub revert_reason: Option<String>,
    /// Maps a broken invariant to its specific error.
    pub errors: HashMap<String, InvariantFuzzError>,
}

impl InvariantFailures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_inner(self) -> (usize, HashMap<String, InvariantFuzzError>) {
        (self.reverts, self.errors)
    }

    pub fn record_failure(&mut self, invariant: &Function, failure: InvariantFuzzError) {
        self.errors.insert(invariant.name.clone(), failure);
    }

    pub fn has_failure(&self, invariant: &Function) -> bool {
        self.errors.contains_key(&invariant.name)
    }

    pub fn get_failure(&self, invariant: &Function) -> Option<&InvariantFuzzError> {
        self.errors.get(&invariant.name)
    }

    pub fn can_continue(&self, invariants: usize) -> bool {
        self.errors.len() < invariants
    }
}

impl fmt::Display for InvariantFailures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        writeln!(f, "      ❌ Failures: {}", self.errors.len())?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum InvariantFuzzError {
    Revert(FailedInvariantCaseData),
    BrokenInvariant(FailedInvariantCaseData),
    MaxAssumeRejects(u32),
}

impl InvariantFuzzError {
    pub fn revert_reason(&self) -> Option<String> {
        match self {
            Self::BrokenInvariant(case_data) | Self::Revert(case_data) => {
                (!case_data.revert_reason.is_empty()).then(|| case_data.revert_reason.clone())
            }
            Self::MaxAssumeRejects(allowed) => {
                Some(format!("`vm.assume` rejected too many inputs ({allowed} allowed)"))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct FailedInvariantCaseData {
    /// The proptest error occurred as a result of a test case.
    pub test_error: TestError<Vec<BasicTxDetails>>,
    /// The return reason of the offending call.
    pub return_reason: Reason,
    /// The revert string of the offending call.
    pub revert_reason: String,
    /// Address of the invariant asserter.
    pub addr: Address,
    /// Function calldata for invariant check.
    pub calldata: Bytes,
    /// Inner fuzzing Sequence coming from overriding calls.
    pub inner_sequence: Vec<Option<BasicTxDetails>>,
    /// Shrink run limit
    pub shrink_run_limit: u32,
    /// Fail on revert, used to check sequence when shrinking.
    pub fail_on_revert: bool,
}

impl FailedInvariantCaseData {
    pub fn new(
        invariant_contract: &InvariantContract<'_>,
        shrink_run_limit: u32,
        fail_on_revert: bool,
        targeted_contracts: &FuzzRunIdentifiedContracts,
        calldata: &[BasicTxDetails],
        call_result: &RawCallResult,
        inner_sequence: &[Option<BasicTxDetails>],
    ) -> Self {
        // Collect abis of fuzzed and invariant contracts to decode custom error.
        let revert_reason = RevertDecoder::new()
            .with_abis(targeted_contracts.targets.lock().values().map(|c| &c.abi))
            .with_abi(invariant_contract.abi)
            .decode(call_result.result.as_ref(), call_result.exit_reason);

        let func = invariant_contract.invariant_fn;
        debug_assert!(func.inputs.is_empty());
        let origin = func.name.as_str();
        Self {
            test_error: TestError::Fail(
                format!("{origin}, reason: {revert_reason}").into(),
                calldata.to_vec(),
            ),
            return_reason: "".into(),
            revert_reason,
            addr: invariant_contract.address,
            calldata: func.selector().to_vec().into(),
            inner_sequence: inner_sequence.to_vec(),
            shrink_run_limit,
            fail_on_revert,
        }
    }
}
