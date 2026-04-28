use super::InvariantContract;
use crate::executors::RawCallResult;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, Selector};
use foundry_evm_core::{
    decode::{ASSERTION_FAILED_PREFIX, EMPTY_REVERT_DATA, RevertDecoder},
    evm::FoundryEvmNetwork,
};
use foundry_evm_fuzz::{BasicTxDetails, Reason, invariant::FuzzRunIdentifiedContracts};
use proptest::test_runner::TestError;
use std::{collections::HashMap, fmt};

/// Records a single handler-side assertion bug discovered during an invariant campaign.
///
/// Handler-side assertions (e.g. a `require`/`assert` inside a fuzzed handler that the campaign
/// reaches with a malformed input) are bugs in their own right, but they are *not* invariant
/// predicate violations. We record them once per `(reverter, selector)` so the campaign can keep
/// running for the rest of the budget and surface deeper bugs without polluting the invariant
/// `errors` map or stopping the run.
#[derive(Clone, Debug)]
pub struct HandlerAssertionFailure {
    /// Address of the handler contract whose call asserted/reverted with an assertion.
    pub reverter: Address,
    /// 4-byte selector of the failing handler function.
    pub selector: Selector,
    /// Full call sequence leading up to (and including) the failing call.
    pub call_sequence: Vec<BasicTxDetails>,
    /// Decoded revert/assert reason.
    pub revert_reason: String,
    /// Always `true` for entries in this struct; mirrored for symmetry with
    /// [`FailedInvariantCaseData::assertion_failure`].
    pub assertion_failure: bool,
}

/// Stores information about failures and reverts of the invariant tests.
#[derive(Clone, Default)]
pub struct InvariantFailures {
    /// Total number of reverts.
    pub reverts: usize,
    /// The latest revert reason of a run.
    pub revert_reason: Option<String>,
    /// Maps a broken invariant to its specific error.
    pub errors: HashMap<String, InvariantFuzzError>,
    /// Handler-side assertion bugs discovered during the campaign, keyed by
    /// `(reverter, selector)` so each unique handler bug is recorded once.
    pub broken_handlers: HashMap<(Address, Selector), HandlerAssertionFailure>,
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
        debug_assert!(invariants > 0, "invariant_fns must not be empty");
        self.errors.len() < invariants
    }

    /// Records a handler-side assertion bug. The first occurrence for a given
    /// `(reverter, selector)` wins; subsequent calls are no-ops to keep the report tidy.
    pub fn record_handler_failure(
        &mut self,
        key: (Address, Selector),
        failure: HandlerAssertionFailure,
    ) {
        self.broken_handlers.entry(key).or_insert(failure);
    }

    /// Returns true if a handler-side assertion bug has already been recorded for the given
    /// target/selector pair.
    pub fn has_handler_failure(&self, target: Address, selector: Selector) -> bool {
        self.broken_handlers.contains_key(&(target, selector))
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
    /// Whether this failure originated from a handler assertion.
    pub assertion_failure: bool,
}

impl FailedInvariantCaseData {
    pub fn new<FEN: FoundryEvmNetwork>(
        invariant_contract: &InvariantContract<'_>,
        shrink_run_limit: u32,
        fail_on_revert: bool,
        targeted_contracts: &FuzzRunIdentifiedContracts,
        calldata: &[BasicTxDetails],
        call_result: RawCallResult<FEN>,
        inner_sequence: &[Option<BasicTxDetails>],
    ) -> Self {
        // Collect abis of fuzzed and invariant contracts to decode custom error.
        let revert_reason = RevertDecoder::new()
            .with_abis(targeted_contracts.targets.lock().values().map(|c| &c.abi))
            .with_abi(invariant_contract.abi)
            .decode(call_result.result.as_ref(), call_result.exit_reason);
        // Non-reverting assertion failures surface through Foundry's failure flags instead of
        // revert data. Use a stable fallback so invariant output is not blank.
        let revert_reason =
            if !call_result.reverted && matches!(revert_reason.as_str(), "" | EMPTY_REVERT_DATA) {
                ASSERTION_FAILED_PREFIX.to_string()
            } else {
                revert_reason
            };

        let func = invariant_contract.primary_invariant_fn;
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
            assertion_failure: false,
        }
    }

    /// Marks this case as assertion-originated and normalizes empty decoded revert data from
    /// non-reverting assertion paths into a stable user-facing message.
    pub fn with_assertion_failure(mut self, assertion_failure: bool) -> Self {
        self.assertion_failure = assertion_failure;
        if assertion_failure && matches!(self.revert_reason.as_str(), "" | EMPTY_REVERT_DATA) {
            self.revert_reason = ASSERTION_FAILED_PREFIX.to_string();
        }
        self
    }
}
