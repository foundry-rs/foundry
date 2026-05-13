use super::InvariantContract;
use crate::executors::RawCallResult;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes};
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    decode::{ASSERTION_FAILED_PREFIX, EMPTY_REVERT_DATA, RevertDecoder},
    evm::FoundryEvmNetwork,
};
use foundry_evm_fuzz::{BasicTxDetails, Reason, invariant::FuzzRunIdentifiedContracts};
use proptest::test_runner::TestError;
use std::{collections::HashMap, fmt};

/// Run-scoped context bundling the references that an invariant run needs in multiple places
/// (recording failures, attributing breaks to a specific invariant, etc.).
///
/// Constructed once per loop iteration and passed by reference; produces failure records via
/// [`InvariantRunCtx::failed_case`].
pub struct InvariantRunCtx<'a> {
    /// The invariant test contract definition.
    pub contract: &'a InvariantContract<'a>,
    /// Active invariant configuration (provides `shrink_run_limit`, `fail_on_revert`, ...).
    pub config: &'a InvariantConfig,
    /// Fuzz targets discovered for this run.
    pub targeted_contracts: &'a FuzzRunIdentifiedContracts,
    /// Inputs of the current run, used as the failing call sequence.
    pub calldata: &'a [BasicTxDetails],
}

impl<'a> InvariantRunCtx<'a> {
    /// Builds a [`FailedInvariantCaseData`] attributed to `broken_fn`.
    ///
    /// `fail_on_revert` is taken separately because `assert_invariants` overrides it with
    /// the per-invariant flag, while every other call site forwards `self.config.fail_on_revert`.
    /// `assertion_failure` is set when the failure originated from a Solidity `assert`/
    /// `vm.assert*` path; it normalizes empty decoded revert data into a stable user-facing
    /// message so invariant output is not blank.
    pub fn failed_case<FEN: FoundryEvmNetwork>(
        &self,
        broken_fn: &Function,
        fail_on_revert: bool,
        assertion_failure: bool,
        call_result: RawCallResult<FEN>,
        inner_sequence: &[Option<BasicTxDetails>],
    ) -> FailedInvariantCaseData {
        // Collect abis of fuzzed and invariant contracts to decode custom error.
        let revert_reason = RevertDecoder::new()
            .with_abis(self.targeted_contracts.targets.lock().values().map(|c| &c.abi))
            .with_abi(self.contract.abi)
            .decode(call_result.result.as_ref(), call_result.exit_reason);
        // Non-reverting assertion failures surface through Foundry's failure flags instead of
        // revert data. Use a stable fallback so invariant output is not blank, both for the
        // successful-call/assertion path and the explicit assertion_failure flag.
        let needs_fallback = matches!(revert_reason.as_str(), "" | EMPTY_REVERT_DATA);
        let revert_reason = if needs_fallback && (!call_result.reverted || assertion_failure) {
            ASSERTION_FAILED_PREFIX.to_string()
        } else {
            revert_reason
        };

        let origin = broken_fn.name.as_str();
        FailedInvariantCaseData {
            test_error: TestError::Fail(
                format!("{origin}, reason: {revert_reason}").into(),
                self.calldata.to_vec(),
            ),
            return_reason: "".into(),
            revert_reason,
            addr: self.contract.address,
            calldata: broken_fn.selector().to_vec().into(),
            inner_sequence: inner_sequence.to_vec(),
            shrink_run_limit: self.config.shrink_run_limit,
            fail_on_revert,
            assertion_failure,
        }
    }
}

/// Stores information about failures and reverts of the invariant tests.
#[derive(Clone, Default)]
pub struct InvariantFailures {
    /// Total number of reverts.
    pub reverts: usize,
    /// The latest revert reason of a run.
    pub revert_reason: Option<String>,
    /// Maps each broken invariant (by function name) to its specific error.
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

    /// Returns the recorded revert reason for `invariant`, or an empty string if the invariant
    /// has no recorded failure (or its failure carries no reason). Used when emitting failure
    /// events so the metrics payload mirrors the persisted failure.
    pub fn broken_reason(&self, invariant: &Function) -> String {
        self.get_failure(invariant).and_then(|e| e.revert_reason()).unwrap_or_default()
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
    /// Whether this failure originated from a handler assertion.
    pub assertion_failure: bool,
}
