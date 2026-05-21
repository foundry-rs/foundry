use super::InvariantContract;
use crate::executors::RawCallResult;
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, Selector, keccak256};
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    decode::{ASSERTION_FAILED_PREFIX, EMPTY_REVERT_DATA, RevertDecoder},
    evm::FoundryEvmNetwork,
};
use foundry_evm_fuzz::{BasicTxDetails, Reason, invariant::FuzzRunIdentifiedContracts};
use proptest::test_runner::TestError;
use std::{collections::HashMap, fmt};

/// A handler-side assertion bug: a `require`/`assert` inside a fuzzed handler that the
/// campaign reached. Deduped by `(reverter, selector)` site (Echidna/Medusa semantics),
/// shortest sequence wins on collision.
#[derive(Clone, Debug)]
pub struct HandlerAssertionFailure {
    /// Handler contract whose call asserted.
    pub reverter: Address,
    /// 4-byte selector of the failing function.
    pub selector: Selector,
    /// Call sequence including the failing call (post-shrink: minimal prefix).
    pub call_sequence: Vec<BasicTxDetails>,
    /// Pre-shrink length, for the `(original: N, shrunk: M)` renderer.
    pub original_sequence_len: usize,
    /// Decoded revert/assert reason.
    pub revert_reason: String,
    /// Stable hash of edge coverage at the asserting call (falls back to `(reverter,
    /// selector)`). Used by the shrinker to preserve path identity, not for dedup.
    pub edge_fingerprint: B256,
}

impl HandlerAssertionFailure {
    /// Builds a failure from a replayed sequence whose last call asserted.
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
            edge_fingerprint,
        }
    }
}

/// Run-scoped references shared by failure-recording paths in an invariant run.
pub struct InvariantRunCtx<'a> {
    /// The invariant test contract.
    pub contract: &'a InvariantContract<'a>,
    /// Active invariant configuration.
    pub config: &'a InvariantConfig,
    /// Fuzz targets discovered for this run.
    pub targeted_contracts: &'a FuzzRunIdentifiedContracts,
    /// Inputs of the current run, used as the failing call sequence.
    pub calldata: &'a [BasicTxDetails],
}

impl<'a> InvariantRunCtx<'a> {
    /// Builds a [`FailedInvariantCaseData`] attributed to `broken_fn`. `fail_on_revert` is
    /// passed in because `assert_invariants` overrides it with the per-invariant flag.
    /// `assertion_failure=true` normalizes empty revert data so output is not blank.
    pub fn failed_case<FEN: FoundryEvmNetwork>(
        &self,
        broken_fn: &Function,
        fail_on_revert: bool,
        assertion_failure: bool,
        call_result: RawCallResult<FEN>,
        inner_sequence: &[Option<BasicTxDetails>],
    ) -> FailedInvariantCaseData {
        let revert_reason = self.decode_revert_reason(&call_result, assertion_failure);
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

    /// Decodes the revert/assert reason without allocating a full [`FailedInvariantCaseData`].
    /// Used by callers that only need the reason (e.g. handler-bug recording).
    pub fn decode_revert_reason<FEN: FoundryEvmNetwork>(
        &self,
        call_result: &RawCallResult<FEN>,
        assertion_failure: bool,
    ) -> String {
        let revert_reason = RevertDecoder::new()
            .with_abis(self.targeted_contracts.targets().values().map(|c| &c.abi))
            .with_abi(self.contract.abi)
            .decode(call_result.result.as_ref(), call_result.exit_reason);
        // Non-reverting assertion failures surface through Foundry's failure flags, not
        // revert data — fall back so invariant output is not blank.
        let needs_fallback = matches!(revert_reason.as_str(), "" | EMPTY_REVERT_DATA);
        if needs_fallback && (!call_result.reverted || assertion_failure) {
            ASSERTION_FAILED_PREFIX.to_string()
        } else {
            revert_reason
        }
    }
}

/// Edge-coverage fingerprint for a handler-side assertion call. Prefers a pre-merge
/// edges hash; falls back to `keccak(target || selector)` when edge coverage is disabled.
pub fn handler_edge_fingerprint(
    pre_merge_edges_hash: Option<B256>,
    target: Address,
    selector: Selector,
) -> B256 {
    if let Some(hash) = pre_merge_edges_hash {
        return hash;
    }
    let mut buf = [0u8; 24];
    buf[..20].copy_from_slice(target.as_slice());
    buf[20..].copy_from_slice(selector.as_slice());
    keccak256(buf)
}

/// Records a handler-side assertion bug (if strictly shorter than the existing repro for
/// this site) and pops the just-asserted reverted input from `inputs`. Shared by the
/// periodic-check path and the inline check-skipped path.
#[expect(clippy::too_many_arguments)]
pub(crate) fn record_handler_assertion_bug<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    config: &InvariantConfig,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    failures: &mut InvariantFailures,
    inputs: &mut Vec<BasicTxDetails>,
    handler_target: Address,
    handler_selector: Selector,
    pre_merge_edges_hash: Option<B256>,
    call_result: RawCallResult<FEN>,
    call_reverted: bool,
    is_optimization: bool,
) {
    let fingerprint =
        handler_edge_fingerprint(pre_merge_edges_hash, handler_target, handler_selector);

    if !handler_site_already_minimal(
        &failures.failures,
        (handler_target, handler_selector),
        inputs.len(),
    ) {
        // Handler bugs go through `FailureKey::Handler`; we only need the reason.
        let revert_reason = InvariantRunCtx {
            contract: invariant_contract,
            config,
            targeted_contracts,
            calldata: inputs,
        }
        .decode_revert_reason(&call_result, true);
        let call_sequence = inputs.clone();
        let original_sequence_len = call_sequence.len();
        failures.record_handler_failure(HandlerAssertionFailure {
            reverter: handler_target,
            selector: handler_selector,
            call_sequence,
            original_sequence_len,
            revert_reason,
            edge_fingerprint: fingerprint,
        });
    }

    // Standard reverted-input pop. Delay-enabled campaigns keep reverted calls so
    // shrinking can preserve their warp/roll contribution.
    if call_reverted && !is_optimization && !config.has_delay() {
        inputs.pop();
    }
}

/// True iff there is already a [`HandlerAssertionFailure`] for `site` no longer than
/// `candidate_len`. Used to skip inserting a not-strictly-shorter repro.
pub fn handler_site_already_minimal(
    failures: &HashMap<FailureKey, InvariantFuzzError>,
    site: (Address, Selector),
    candidate_len: usize,
) -> bool {
    failures
        .get(&FailureKey::Handler(site.0, site.1))
        .and_then(InvariantFuzzError::as_handler_assertion)
        .is_some_and(|existing| existing.call_sequence.len() <= candidate_len)
}

/// Stable hash of the call's edge coverage, taken *before* `merge_edge_coverage`
/// zeroes the buffer. Returns `None` when edge coverage is disabled.
pub fn snapshot_edge_fingerprint<FEN: FoundryEvmNetwork>(
    call_result: &RawCallResult<FEN>,
) -> Option<B256> {
    let edges = call_result.edge_coverage.as_deref()?;
    if edges.is_empty() || edges.iter().all(|b| *b == 0) {
        return None;
    }
    Some(keccak256(edges))
}

/// Identifies a single entry in the [`InvariantFailures`] map. Invariant predicate
/// failures and handler-side assertion bugs share one map keyed by this enum.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum FailureKey {
    /// Keyed by invariant function name.
    Invariant(String),
    /// Keyed by handler `(reverter, selector)` site (Echidna/Medusa semantics: one bug
    /// per handler function regardless of code path).
    Handler(Address, Selector),
}

/// Stores invariant test failures and revert counts.
///
/// TODO: dedup multiple distinct `assert(...)` within the same `(reverter, selector)`
/// handler if callers ever need finer attribution (e.g. per-assertion-label).
#[derive(Clone, Default)]
pub struct InvariantFailures {
    /// Total number of reverts.
    pub reverts: usize,
    /// Invariant predicate failures and handler-side assertion bugs share one map.
    /// Mutate only via `record_failure` / `record_handler_failure` / `seed_handler_failure`
    /// so the cached counters stay in sync.
    pub(crate) failures: HashMap<FailureKey, InvariantFuzzError>,
    /// Cached `FailureKey::Invariant` count, kept O(1) on the hot path.
    invariant_count: usize,
    /// Cached `FailureKey::Handler` count, read on progress/metrics ticks.
    handler_count: usize,
}

impl InvariantFailures {
    pub fn new() -> Self {
        Self::default()
    }

    /// Splits `self.failures` into the legacy `(invariant_errors, handler_errors)` pair.
    pub fn partition(
        self,
    ) -> (HashMap<String, InvariantFuzzError>, HashMap<(Address, Selector), InvariantFuzzError>)
    {
        let mut invariant_errors = HashMap::new();
        let mut handler_errors = HashMap::new();
        for (key, err) in self.failures {
            match key {
                FailureKey::Invariant(name) => {
                    invariant_errors.insert(name, err);
                }
                FailureKey::Handler(addr, sel) => {
                    handler_errors.insert((addr, sel), err);
                }
            }
        }
        (invariant_errors, handler_errors)
    }

    pub fn record_failure(&mut self, invariant: &Function, failure: InvariantFuzzError) {
        let prev = self.failures.insert(FailureKey::Invariant(invariant.name.clone()), failure);
        if prev.is_none() {
            self.invariant_count += 1;
        }
    }

    pub fn has_failure(&self, invariant: &Function) -> bool {
        self.failures.contains_key(&FailureKey::Invariant(invariant.name.clone()))
    }

    pub fn get_failure(&self, invariant: &Function) -> Option<&InvariantFuzzError> {
        self.failures.get(&FailureKey::Invariant(invariant.name.clone()))
    }

    /// Recorded revert reason for `invariant`, or empty when none. Used by failure events
    /// so the metrics payload mirrors the persisted failure.
    pub fn broken_reason(&self, invariant: &Function) -> String {
        self.get_failure(invariant).and_then(|e| e.revert_reason()).unwrap_or_default()
    }

    pub const fn can_continue(&self, invariants: usize) -> bool {
        self.invariant_count() < invariants
    }

    /// Number of unique broken invariant predicates (O(1), cached).
    pub const fn invariant_count(&self) -> usize {
        self.invariant_count
    }

    /// Number of unique handler-side assertion bugs (O(1), cached).
    pub const fn handler_count(&self) -> usize {
        self.handler_count
    }

    /// Records a handler-side assertion bug. Deduped by `(reverter, selector)` site;
    /// shortest sequence wins on collision.
    pub fn record_handler_failure(&mut self, failure: HandlerAssertionFailure) {
        let site = (failure.reverter, failure.selector);
        if !handler_site_already_minimal(&self.failures, site, failure.call_sequence.len()) {
            let prev = self.failures.insert(
                FailureKey::Handler(site.0, site.1),
                InvariantFuzzError::HandlerAssertion(failure),
            );
            if prev.is_none() {
                self.handler_count += 1;
            }
        }
    }

    /// Inserts a persisted-replay handler bug. Skips dedup (caller seeds an empty map)
    /// but bumps `handler_count` so the live counter is correct from the first tick.
    pub fn seed_handler_failure(
        &mut self,
        target: Address,
        selector: Selector,
        err: InvariantFuzzError,
    ) {
        let prev = self.failures.insert(FailureKey::Handler(target, selector), err);
        if prev.is_none() {
            self.handler_count += 1;
        }
    }

    /// Returns true if a handler bug has already been recorded for the given site.
    pub fn has_handler_failure(&self, target: Address, selector: Selector) -> bool {
        self.failures.contains_key(&FailureKey::Handler(target, selector))
    }

    /// Mutable iterator over handler-side assertion bug entries (post-campaign shrink loop).
    pub fn handler_failures_mut(
        &mut self,
    ) -> impl Iterator<Item = ((Address, Selector), &mut InvariantFuzzError)> {
        self.failures.iter_mut().filter_map(|(key, err)| match key {
            FailureKey::Handler(addr, sel) => Some(((*addr, *sel), err)),
            FailureKey::Invariant(_) => None,
        })
    }
}

impl fmt::Display for InvariantFailures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        writeln!(f, "      ❌ Failures: {}", self.invariant_count())?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum InvariantFuzzError {
    /// A handler call reverted under `fail_on_revert = true`.
    Revert(FailedInvariantCaseData),
    /// An `invariant_*` predicate returned `false` (or asserted).
    BrokenInvariant(FailedInvariantCaseData),
    /// A handler-side `assert(...)` / `vm.assert*` failed (bug inside a handler, not in
    /// an `invariant_*` predicate). Recorded per `(reverter, selector)` site.
    HandlerAssertion(HandlerAssertionFailure),
    /// `vm.assume` rejected more inputs than allowed.
    MaxAssumeRejects(u32),
}

impl InvariantFuzzError {
    pub fn revert_reason(&self) -> Option<String> {
        match self {
            Self::BrokenInvariant(case_data) | Self::Revert(case_data) => {
                (!case_data.revert_reason.is_empty()).then(|| case_data.revert_reason.clone())
            }
            Self::HandlerAssertion(failure) => {
                (!failure.revert_reason.is_empty()).then(|| failure.revert_reason.clone())
            }
            Self::MaxAssumeRejects(allowed) => {
                Some(format!("`vm.assume` rejected too many inputs ({allowed} allowed)"))
            }
        }
    }

    /// Wrapped `HandlerAssertionFailure` if this is the [`Self::HandlerAssertion`] variant.
    pub const fn as_handler_assertion(&self) -> Option<&HandlerAssertionFailure> {
        match self {
            Self::HandlerAssertion(failure) => Some(failure),
            _ => None,
        }
    }

    /// Mutable counterpart of [`Self::as_handler_assertion`]. Used by post-campaign shrinking.
    pub const fn as_handler_assertion_mut(&mut self) -> Option<&mut HandlerAssertionFailure> {
        match self {
            Self::HandlerAssertion(failure) => Some(failure),
            _ => None,
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
