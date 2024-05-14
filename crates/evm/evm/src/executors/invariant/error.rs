use super::{BasicTxDetails, InvariantContract};
use crate::executors::RawCallResult;
use alloy_primitives::{Address, Bytes};
use foundry_config::InvariantConfig;
use foundry_evm_core::decode::RevertDecoder;
use foundry_evm_fuzz::{invariant::FuzzRunIdentifiedContracts, Reason};
use proptest::test_runner::TestError;

/// Stores information about failures and reverts of the invariant tests.
#[derive(Clone, Default)]
pub struct InvariantFailures {
    /// Total number of reverts.
    pub reverts: usize,
    /// How many different invariants have been broken.
    pub broken_invariants_count: usize,
    /// The latest revert reason of a run.
    pub revert_reason: Option<String>,
    /// Maps a broken invariant to its specific error.
    pub error: Option<InvariantFuzzError>,
}

impl InvariantFailures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_inner(self) -> (usize, Option<InvariantFuzzError>) {
        (self.reverts, self.error)
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
            Self::MaxAssumeRejects(allowed) => Some(format!(
                "The `vm.assume` cheatcode rejected too many inputs ({allowed} allowed)"
            )),
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
    /// Function data for invariant check.
    pub func: Bytes,
    /// Inner fuzzing Sequence coming from overriding calls.
    pub inner_sequence: Vec<Option<BasicTxDetails>>,
    /// Shrink run limit
    pub shrink_run_limit: usize,
    /// Fail on revert, used to check sequence when shrinking.
    pub fail_on_revert: bool,
}

impl FailedInvariantCaseData {
    pub fn new(
        invariant_contract: &InvariantContract<'_>,
        invariant_config: &InvariantConfig,
        targeted_contracts: &FuzzRunIdentifiedContracts,
        calldata: &[BasicTxDetails],
        call_result: RawCallResult,
        inner_sequence: &[Option<BasicTxDetails>],
    ) -> Self {
        // Collect abis of fuzzed and invariant contracts to decode custom error.
        let targets = targeted_contracts.targets.lock();
        let abis = targets
            .iter()
            .map(|contract| &contract.1 .1)
            .chain(std::iter::once(invariant_contract.abi));

        let revert_reason = RevertDecoder::new()
            .with_abis(abis)
            .decode(call_result.result.as_ref(), Some(call_result.exit_reason));

        let func = invariant_contract.invariant_function;
        let origin = func.name.as_str();
        Self {
            test_error: proptest::test_runner::TestError::Fail(
                format!("{origin}, reason: {revert_reason}").into(),
                calldata.to_vec(),
            ),
            return_reason: "".into(),
            revert_reason,
            addr: invariant_contract.address,
            func: func.selector().to_vec().into(),
            inner_sequence: inner_sequence.to_vec(),
            shrink_run_limit: invariant_config.shrink_run_limit,
            fail_on_revert: invariant_config.fail_on_revert,
        }
    }
}
