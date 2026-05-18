//! Seed and frontier selection for the symbolic-assist worker.
//!
//! Both helpers are intentionally cheap and *advisory*: they read snapshots
//! of corpus state and never mutate it. Heavy work (replay, validation) is
//! done by `mod.rs`.

use super::types::{FrontierKey, SymExecState};
use crate::inspectors::{BranchObservation, BranchTrace, CmpKind};
use alloy_primitives::map::DefaultHashBuilder;
use foundry_evm_fuzz::BasicTxDetails;

/// A read-only snapshot of a corpus entry, just enough to pick a seed.
#[derive(Clone, Debug)]
pub struct SeedSnapshot {
    pub uuid: uuid::Uuid,
    pub tx_seq: Vec<BasicTxDetails>,
    pub is_favored: bool,
    pub new_finds_produced: usize,
}

/// Score a seed for symbolic exploration. Higher is better.
///
/// v1 scoring (intentionally simple):
/// 1. favored seeds first
/// 2. seeds with more `new_finds_produced` (proven productive)
/// 3. shorter sequences (cheaper to replay & easier to mutate)
///
/// We do *not* yet penalize seeds whose frontiers are exhausted — that
/// happens at frontier-selection time.
pub fn score_seed(seed: &SeedSnapshot) -> i64 {
    let len_penalty = seed.tx_seq.len().min(64) as i64;
    let mut score: i64 = 0;
    if seed.is_favored {
        score += 1_000;
    }
    score += (seed.new_finds_produced.min(1_000) as i64) * 10;
    score -= len_penalty;
    score
}

/// Pick the highest-scoring seed from a small candidate pool.
///
/// Caller is expected to pre-filter / sample so we don't score the entire
/// corpus on every cycle.
pub fn pick_seed(candidates: &[SeedSnapshot]) -> Option<&SeedSnapshot> {
    candidates.iter().max_by_key(|s| score_seed(s))
}

/// Pick a frontier from a replay trace.
///
/// Strategy:
/// - prefer the *deepest* observation whose opposite edge is currently unseen (the seed already
///   satisfies all earlier guards),
/// - skip frontiers that have hit `MAX_FRONTIER_ATTEMPTS`,
/// - skip frontiers without a recoverable compare (v1 has no symbolic reasoning beyond ABI rewrites
///   of compare operands).
pub fn pick_frontier<F>(
    trace: &BranchTrace,
    tx_index: u32,
    selector: [u8; 4],
    state: &SymExecState,
    history_map: &[u8],
    hash_builder: &DefaultHashBuilder,
    is_unseen: F,
) -> Option<(FrontierKey, BranchObservation)>
where
    F: Fn(&[u8], usize) -> bool,
{
    collect_frontiers(trace, tx_index, selector, state, history_map, hash_builder, is_unseen, 1)
        .into_iter()
        .next()
}

/// Collect up to `limit` eligible frontiers from the trace, deepest first.
/// Used by the assist loop to fan out a single replay across several
/// candidate frontiers when the deepest one is uninteresting (e.g. it
/// lives in test-harness post-call code).
#[allow(clippy::too_many_arguments)]
pub fn collect_frontiers<F>(
    trace: &BranchTrace,
    tx_index: u32,
    selector: [u8; 4],
    state: &SymExecState,
    history_map: &[u8],
    hash_builder: &DefaultHashBuilder,
    is_unseen: F,
    limit: usize,
) -> Vec<(FrontierKey, BranchObservation)>
where
    F: Fn(&[u8], usize) -> bool,
{
    let mut out = Vec::new();
    for obs in trace.branches.iter().rev() {
        if out.len() >= limit {
            break;
        }
        // No compare → v1 has no targeted mutation to offer.
        let Some(cmp) = obs.cmp else { continue };

        // Skip trivially uninvertible frontiers — equality-class compares
        // where both operands are already equal (e.g. `EQ(0, 0)` emitted
        // by Solidity's runtime cleanup) cannot be flipped by rewriting a
        // single calldata word.
        if matches!(cmp.kind, CmpKind::Eq) && cmp.lhs == cmp.rhs {
            continue;
        }

        let frontier_id = obs.frontier_edge_id(hash_builder);
        if !is_unseen(history_map, frontier_id) {
            continue;
        }

        let other_dest = if obs.took_branch { obs.other_dest } else { obs.taken_dest };
        let key = FrontierKey {
            address: obs.address,
            pc: obs.pc,
            other_dest_lo: other_dest.as_limbs()[0],
            tx_index,
            selector,
        };

        if state.should_try(&key) {
            out.push((key, obs.clone()));
        }
    }
    out
}

/// Default predicate: an edge is "unseen" if its hitcount in the history map
/// is zero. The corpus binning logic uses non-linear bins for *new coverage*
/// detection, but for "have we ever taken this edge" plain `== 0` is correct.
pub fn unseen_in_history(history_map: &[u8], edge_id: usize) -> bool {
    history_map.get(edge_id).copied().unwrap_or(0) == 0
}
