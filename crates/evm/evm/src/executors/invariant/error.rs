use super::InvariantContract;
use crate::executors::RawCallResult;
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, Selector, keccak256};
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
/// predicate violations. We dedup them by the `(reverter, selector)` site of the asserting
/// call so the same handler function asserting via N different code paths counts as a single
/// bug (Echidna/Medusa semantics). The shortest call sequence wins on collision, so persisted
/// reproducers stay minimal. `edge_fingerprint` is still recorded on the failure value to let
/// the shrinker preserve path identity when minimizing a single reproducer.
#[derive(Clone, Debug)]
pub struct HandlerAssertionFailure {
    /// Address of the handler contract whose call asserted/reverted with an assertion.
    pub reverter: Address,
    /// 4-byte selector of the failing handler function.
    pub selector: Selector,
    /// Full call sequence leading up to (and including) the failing call. After shrinking
    /// this holds the minimal prefix that still triggers the anchor assertion.
    pub call_sequence: Vec<BasicTxDetails>,
    /// Pre-shrink length of `call_sequence`, used by the renderer's
    /// `(original: N, shrunk: M)` output.
    pub original_sequence_len: usize,
    /// Decoded revert/assert reason.
    pub revert_reason: String,
    /// Always `true` for entries in this struct; mirrored for symmetry with
    /// `FailedInvariantCaseData::assertion_failure`.
    pub assertion_failure: bool,
    /// Stable hash of the asserting call's edge coverage (or `(reverter, selector)` when
    /// edge coverage is unavailable). Not used for dedup (see `InvariantFailures.broken_handlers`)
    /// but kept so the shrinker can preserve path identity when minimizing this reproducer.
    pub edge_fingerprint: B256,
}

impl HandlerAssertionFailure {
    /// Builds a failure from a replayed sequence whose last call asserted; `(reverter,
    /// selector)` are derived from that call's `(target, calldata[..4])`.
    pub fn from_replayed_sequence(
        call_sequence: Vec<BasicTxDetails>,
        edge_fingerprint: B256,
        revert_reason: String,
    ) -> Self {
        let last = call_sequence.last().expect("replayed sequence is non-empty");
        let reverter = last.call_details.target;
        let selector_bytes: [u8; 4] =
            last.call_details.calldata.get(..4).and_then(|s| s.try_into().ok()).unwrap_or_default();
        let original_sequence_len = call_sequence.len();
        Self {
            reverter,
            selector: Selector::from(selector_bytes),
            call_sequence,
            original_sequence_len,
            revert_reason,
            assertion_failure: true,
            edge_fingerprint,
        }
    }
}

/// Computes the edge-coverage fingerprint for a handler-side assertion call. `target` is
/// the handler contract address whose call asserted/reverted (mirrors
/// `HandlerAssertionFailure::reverter`).
///
/// Prefers `pre_merge_edges_hash` (a hash of the call's edge coverage taken *before*
/// `merge_edge_coverage` zeroes the buffer). Falls back to a `(target, selector)` hash so
/// the dedup key is always defined and behavior degrades gracefully when edge coverage
/// collection is disabled.
pub fn handler_edge_fingerprint(
    pre_merge_edges_hash: Option<B256>,
    target: Address,
    selector: Selector,
) -> B256 {
    if let Some(hash) = pre_merge_edges_hash {
        return hash;
    }
    // Fallback: stable hash of (target || selector). Preserves prior key-based dedup.
    let mut buf = [0u8; 24];
    buf[..20].copy_from_slice(target.as_slice());
    buf[20..].copy_from_slice(selector.as_slice());
    keccak256(buf)
}

/// Snapshots the asserting call's edge coverage as a stable hash *before* the corpus's
/// `merge_edge_coverage` zeroes the buffer. Returns `None` when edge coverage is unavailable
/// (e.g. corpus / coverage collection disabled).
pub fn snapshot_edge_fingerprint<FEN: FoundryEvmNetwork>(
    call_result: &RawCallResult<FEN>,
) -> Option<B256> {
    let edges = call_result.edge_coverage.as_deref()?;
    if edges.is_empty() || edges.iter().all(|b| *b == 0) {
        return None;
    }
    Some(keccak256(edges))
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
    /// Handler-side assertion bugs discovered during the campaign, keyed by the
    /// `(reverter, selector)` site of the asserting call. The same handler function
    /// asserting via N different code paths counts as a single bug (Echidna/Medusa
    /// semantics). On collision, the entry with the shortest `call_sequence` wins so
    /// persisted reproducers stay minimal.
    ///
    /// TODO: dedup multiple distinct `assert(...)` failures within the same
    /// `(reverter, selector)` handler. Echidna explicitly cannot tell them apart and
    /// our current key collapses them as well; if/when callers need finer attribution
    /// (e.g. per-assertion-label), extend the key with a stable per-call discriminator
    /// (revert reason hash, source location, or label string).
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

    /// Records a handler-side assertion bug. Keyed by the `(reverter, selector)` site of
    /// the failing call, so the same handler function asserting via different code paths
    /// counts as a single bug. On collision the shortest `call_sequence` wins, giving us
    /// a smaller reproducer over time.
    pub fn record_handler_failure(&mut self, failure: HandlerAssertionFailure) {
        let key = (failure.reverter, failure.selector);
        match self.broken_handlers.get(&key) {
            Some(existing) if existing.call_sequence.len() <= failure.call_sequence.len() => {
                // Existing repro is at least as short; keep it.
            }
            _ => {
                self.broken_handlers.insert(key, failure);
            }
        }
    }

    /// Returns true if a handler bug has already been recorded for the given site.
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
