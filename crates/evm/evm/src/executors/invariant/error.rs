use super::InvariantContract;
use crate::executors::RawCallResult;
use alloy_primitives::{Address, Bytes};
use foundry_config::InvariantConfig;
use foundry_evm_core::decode::RevertDecoder;
use foundry_evm_fuzz::{BasicTxDetails, Reason, invariant::FuzzRunIdentifiedContracts};
use proptest::test_runner::TestError;
use std::collections::BTreeMap;

/// Stores information about failures and reverts of the invariant tests.
#[derive(Clone, Default)]
pub struct InvariantFailures {
    /// Total number of reverts.
    pub reverts: usize,
    /// The latest revert reason of a run.
    pub revert_reason: Option<String>,
    /// Maps a broken invariant to its specific error.
    pub error: Option<InvariantFuzzError>,
    /// Distinct handler-level assertion failures observed during the campaign.
    pub assertion_failures: BTreeMap<String, FailedInvariantCaseData>,
}

impl InvariantFailures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_inner(self) -> (usize, Option<InvariantFuzzError>) {
        (self.reverts, self.error)
    }

    pub fn record_assertion_failure(&mut self, case_data: FailedInvariantCaseData) {
        let key =
            case_data.failing_handler.clone().unwrap_or_else(|| "unknown_handler".to_string());
        self.assertion_failures.entry(key).or_insert(case_data);
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
    /// Fail on Solidity assert failures, used to check sequence when shrinking.
    pub fail_on_assert: bool,
    /// Handler function that triggered a fail-on-assert violation, when available.
    pub failing_handler: Option<String>,
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
        let revert_reason = RevertDecoder::new()
            .with_abis(targeted_contracts.targets.lock().values().map(|c| &c.abi))
            .with_abi(invariant_contract.abi)
            .decode(call_result.result.as_ref(), call_result.exit_reason);

        let func = invariant_contract.invariant_function;
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
            shrink_run_limit: invariant_config.shrink_run_limit,
            fail_on_revert: invariant_config.fail_on_revert,
            fail_on_assert: invariant_config.fail_on_assert,
            failing_handler: None,
        }
    }

    pub fn with_failing_handler(mut self, failing_handler: Option<String>) -> Self {
        self.failing_handler = failing_handler;
        self
    }
}
