//! Symbolic-assist worker.
//!
//! Directed-mutation helper for coverage-guided fuzzing. Each cycle:
//!
//! 1. takes a corpus seed (`Vec<BasicTxDetails>`),
//! 2. replays it concretely under a [`crate::inspectors::BranchTraceInspector`],
//! 3. picks unseen opposite-side branches ("frontiers") from the trace,
//! 4. proposes ABI-aware calldata rewrites that — given the recovered
//!    compare operands — would flip a frontier,
//! 5. validates each candidate through a clone of the live executor,
//!    requiring a real new EVM edge in coverage, and
//! 6. writes accepted candidates to the master worker's `sync/` directory
//!    so the existing corpus protocol distributes them.
//!
//! There is no SMT solver here; the worker has no symbolic engine of its
//! own to feed one, so it can only flip branches whose compare operands
//! are visible at runtime and reachable by rewriting a scalar ABI arg.
//!
//! Scope:
//! - master worker only,
//! - EVM `EdgeCovInspector`-based coverage only (no sancov),
//! - mutates only the final tx of the seed sequence,
//! - skips dynamic ABI types,
//! - hard CPU budget per cycle.

use crate::{
    executors::{Executor, corpus::WorkerCorpus, invariant::execute_tx},
    inspectors::BranchTrace,
};
use alloy_json_abi::Function;
use alloy_primitives::{B256, keccak256, map::DefaultHashBuilder};
use eyre::Result;
use foundry_evm_core::evm::FoundryEvmNetwork;
use foundry_evm_fuzz::{BasicTxDetails, invariant::FuzzRunIdentifiedContracts};

/// Fallback hash builder used when the live executor has no
/// `EdgeCovInspector`. This is only safe to use as a *parity*-with-itself
/// builder — frontier IDs computed against it will not match `history_map`,
/// so the worker only ever produces this when there is nothing to compare
/// against. Real cycles must use [`Executor::inspector().inner.edge_coverage`]'s
/// hash builder instead (see `run_symexec_assist`).
fn fallback_hash_builder() -> DefaultHashBuilder {
    DefaultHashBuilder::default()
}

mod mutate;
mod select;
mod types;

use mutate::propose_calldata_rewrites;
pub use select::SeedSnapshot;
use select::pick_seed;
use types::Candidate;
pub use types::SymExecState;

/// Generate ABI-rewrite candidate sequences from a seed and its branch
/// trace. `tx_index` identifies the call to mutate (always the last one).
///
/// This is the only backend the worker has today; it has no SMT solver
/// and no symbolic engine to feed one, so it can only flip branches whose
/// compare operands are visible at runtime and reachable by rewriting a
/// scalar ABI arg.
#[allow(clippy::too_many_arguments)]
pub fn propose_candidates(
    seed: &[BasicTxDetails],
    trace: &BranchTrace,
    tx_index: usize,
    function: Option<&Function>,
    state: &SymExecState,
    history_map: &[u8],
    hash_builder: &DefaultHashBuilder,
) -> Vec<Candidate> {
    let Some(tx) = seed.get(tx_index) else { return Vec::new() };
    let calldata = &tx.call_details.calldata;
    if calldata.len() < 4 {
        return Vec::new();
    }
    let mut selector = [0u8; 4];
    selector.copy_from_slice(&calldata[..4]);

    // Collect several deepest-first frontiers, not just the very last
    // one — the deepest unseen branch is often in post-call test-harness
    // bookkeeping (e.g. forge-std asserting on the returned bool) and
    // not in the contract under test. Propose calldata rewrites for
    // each so a single replay cycle can flip a guard several frames
    // back along the trace.
    let frontiers = select::collect_frontiers(
        trace,
        tx_index as u32,
        selector,
        state,
        history_map,
        hash_builder,
        select::unseen_in_history,
        MAX_FRONTIERS_PER_CYCLE,
    );
    if frontiers.is_empty() {
        return Vec::new();
    }

    let source_uuid = uuid::Uuid::nil();
    let mut out = Vec::new();
    for (frontier, obs) in frontiers {
        let rewrites = propose_calldata_rewrites(tx, function, &obs);
        for new_tx in rewrites {
            let mut tx_seq = seed.to_vec();
            tx_seq[tx_index] = new_tx;
            out.push(Candidate { tx_seq, frontier, source_uuid });
        }
    }
    out
}

/// Maximum number of distinct frontier branches a single replay cycle is
/// allowed to fan candidate calldata rewrites over. Keeps the per-cycle
/// validation cost bounded while still letting the worker reach guards
/// that live *before* the test-harness post-call code.
const MAX_FRONTIERS_PER_CYCLE: usize = 16;

/// Run a single symbolic-assist cycle on the master worker.
///
/// `function` is the ABI of the call slot the worker is allowed to mutate
/// for stateless fuzz (the worker always mutates the *last* tx of the seed).
/// `targeted_contracts` is used by stateful invariant tests to resolve
/// the final tx's ABI dynamically — pass `None` for stateless fuzz.
/// Exactly one of `function` / `targeted_contracts` must be `Some`.
///
/// `stateful` controls whether prefix txs are committed during replay and
/// validation — `true` for invariant tests, `false` for stateless fuzz.
///
/// Returns the number of candidates accepted into the corpus.
#[tracing::instrument(skip_all)]
pub fn run_symexec_assist<FEN: FoundryEvmNetwork>(
    corpus: &mut WorkerCorpus,
    executor: &Executor<FEN>,
    function: Option<&Function>,
    targeted_contracts: Option<&FuzzRunIdentifiedContracts>,
    state: &mut SymExecState,
    stateful: bool,
) -> Result<usize> {
    if !corpus.is_master() {
        return Ok(0);
    }

    // Snapshot a small candidate pool for seed scoring (avoid scoring the
    // whole corpus on every cycle).
    let pool = corpus.symexec_seed_pool();
    let Some(seed) = pick_seed(&pool).cloned() else { return Ok(0) };

    // 1. Replay the seed with the branch-trace inspector enabled.
    let mut replay_executor = executor.clone();
    replay_executor.inspector_mut().collect_branch_trace(true);
    // Disable edge-coverage on the *replay* executor — branch trace alone
    // is what we need, and we don't want replay to mutate the global edge
    // map.
    replay_executor.inspector_mut().collect_edge_coverage(false);

    // Only the final tx's branches are eligible for mutation, so we
    // only need to *trace* that final tx — the prefix is replayed purely
    // to set up state for stateful tests. Use `execute_tx` (the same
    // helper invariant/fuzz use) so warp, roll and bounded value are
    // honored faithfully — otherwise the replay can diverge from the
    // path the real fuzz loop took.
    let tx_index = seed.tx_seq.len().saturating_sub(1);
    let mut trace = BranchTrace::default();
    for (i, tx) in seed.tx_seq.iter().enumerate() {
        if i == tx_index {
            replay_executor.inspector_mut().collect_branch_trace(true);
        } else {
            replay_executor.inspector_mut().collect_branch_trace(false);
        }
        let mut result = execute_tx(&mut replay_executor, tx)?;
        if i == tx_index
            && let Some(t) = result.branch_trace.take()
        {
            trace.branches.extend(t.branches);
        }
        if stateful && i < tx_index {
            replay_executor.commit(&mut result);
        }
    }
    if trace.is_empty() {
        return Ok(0);
    }

    // 2. Resolve the ABI of the call slot we're allowed to mutate. For invariant tests this is
    //    looked up from the targeted contracts; for stateless fuzz the caller already supplied it.
    let resolved_function: Option<Function> = match (function, targeted_contracts) {
        (Some(f), _) => Some(f.clone()),
        (None, Some(targets)) => seed
            .tx_seq
            .get(tx_index)
            .and_then(|tx| targets.targets.lock().fuzzed_artifacts(tx).1.cloned()),
        (None, None) => None,
    };
    let Some(resolved_function) = resolved_function else {
        return Ok(0);
    };

    // 3. Pick a frontier + propose candidates.
    let history = corpus.history_map_snapshot();
    // CRITICAL: frontier IDs must hash through the *same* `BuildHasher`
    // that `EdgeCovInspector::store_hit` used to populate `history_map`.
    // `DefaultHashBuilder` (alloy's `foldhash::fast::RandomState`) is
    // randomly seeded per instance, so a fresh `::default()` here would
    // index into a completely different bucket than the live history map.
    let hash_builder = executor
        .inspector()
        .inner
        .edge_coverage
        .as_deref()
        .map(|ec| ec.hash_builder().clone())
        .unwrap_or_else(fallback_hash_builder);
    let candidates = propose_candidates(
        &seed.tx_seq,
        &trace,
        tx_index,
        Some(&resolved_function),
        state,
        &history,
        &hash_builder,
    );

    // 4. Validate each candidate and persist accepted ones.
    let mut accepted = 0;
    for mut candidate in candidates {
        candidate.source_uuid = seed.uuid;
        let hash = candidate_hash(&candidate.tx_seq);
        if !state.seen_candidate_hashes.insert(hash) {
            continue;
        }

        let new_edge = corpus.symexec_validate(executor, &candidate.tx_seq, stateful)?;
        state.record_attempt(candidate.frontier, candidate.source_uuid, new_edge);
        if !new_edge {
            continue;
        }

        // Insert directly into the master's in-memory corpus and on
        // disk. We deliberately do NOT also write to `worker0/sync/`:
        // that path is second-resolution and would cause the same
        // candidate to be re-imported (or stale-skipped) by a later
        // `calibrate()` call.
        corpus.insert_symexec_candidate(candidate.tx_seq)?;
        accepted += 1;
    }

    Ok(accepted)
}

/// Stable hash of a candidate sequence — used to skip duplicates we've
/// already validated. In-process state only, so the encoding can change
/// without breaking correctness.
fn candidate_hash(seq: &[BasicTxDetails]) -> B256 {
    let mut bytes = Vec::with_capacity(64);
    for tx in seq {
        bytes.extend_from_slice(tx.sender.as_slice());
        bytes.extend_from_slice(tx.call_details.target.as_slice());
        bytes.extend_from_slice(&tx.call_details.calldata);
    }
    keccak256(&bytes)
}


