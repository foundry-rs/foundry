use crate::executors::{invariant::error::FailedInvariantCaseData, Executor};
use alloy_primitives::U256;
use foundry_evm_core::constants::CALLER;
use foundry_evm_fuzz::invariant::BasicTxDetails;
use proptest::bits::{BitSetLike, VarBitSet};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug)]
struct Shrink {
    call_index: usize,
}

/// Shrinker for a call sequence failure.
/// Iterates sequence call sequence top down and removes calls one by one.
/// If the failure is still reproducible with removed call then moves to the next one.
/// If the failure is not reproducible then restore removed call and moves to next one.
#[derive(Debug)]
struct CallSequenceShrinker {
    /// Length of call sequence to be shrinked.
    call_sequence_len: usize,
    /// Call ids contained in current shrinked sequence.
    included_calls: VarBitSet,
    /// Current shrinked call id.
    shrink: Shrink,
    /// Previous shrinked call id.
    prev_shrink: Option<Shrink>,
}

impl CallSequenceShrinker {
    fn new(call_sequence_len: usize) -> Self {
        Self {
            call_sequence_len,
            included_calls: VarBitSet::saturated(call_sequence_len),
            shrink: Shrink { call_index: 0 },
            prev_shrink: None,
        }
    }

    /// Return candidate shrink sequence to be tested, by removing ids from original sequence.
    fn current(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.call_sequence_len).filter(|&call_id| self.included_calls.test(call_id))
    }

    /// Removes next call from sequence.
    fn simplify(&mut self) -> bool {
        if self.shrink.call_index >= self.call_sequence_len {
            // We reached the end of call sequence, nothing left to simplify.
            false
        } else {
            // Remove current call.
            self.included_calls.clear(self.shrink.call_index);
            // Record current call as previous call.
            self.prev_shrink = Some(self.shrink);
            // Remove next call index
            self.shrink = Shrink { call_index: self.shrink.call_index + 1 };
            true
        }
    }

    /// Reverts removed call from sequence and tries to simplify next call.
    fn complicate(&mut self) -> bool {
        match self.prev_shrink {
            Some(shrink) => {
                // Undo the last call removed.
                self.included_calls.set(shrink.call_index);
                self.prev_shrink = None;
                // Try to simplify next call.
                self.simplify()
            }
            None => false,
        }
    }
}

/// Shrinks the failure case to its smallest sequence of calls.
///
/// Maximal shrinkage is guaranteed if the shrink_run_limit is not set to a value lower than the
/// length of failed call sequence.
///
/// The shrinked call sequence always respect the order failure is reproduced as it is tested
/// top-down.
pub(crate) fn shrink_sequence(
    failed_case: &FailedInvariantCaseData,
    calls: &[BasicTxDetails],
    executor: &Executor,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking.");

    // Special case test: the invariant is *unsatisfiable* - it took 0 calls to
    // break the invariant -- consider emitting a warning.
    let error_call_result =
        executor.call_raw(CALLER, failed_case.addr, failed_case.func.clone(), U256::ZERO)?;
    if error_call_result.reverted {
        return Ok(vec![]);
    }

    let mut shrinker = CallSequenceShrinker::new(calls.len());
    for _ in 0..failed_case.shrink_run_limit {
        // Check candidate sequence result.
        match check_sequence(failed_case, executor.clone(), calls, shrinker.current().collect()) {
            // If candidate sequence still fails then shrink more if possible.
            Ok(false) if !shrinker.simplify() => break,
            // If candidate sequence pass then restore last removed call and shrink other
            // calls if possible.
            Ok(true) if !shrinker.complicate() => break,
            _ => {}
        }
    }

    Ok(shrinker.current().map(|idx| &calls[idx]).cloned().collect())
}

/// Checks if the shrinked sequence fails test, if it does then we can try simplifying more.
fn check_sequence(
    failed_case: &FailedInvariantCaseData,
    mut executor: Executor,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
) -> eyre::Result<bool> {
    let mut sequence_failed = false;
    // Apply the shrinked candidate sequence.
    for call_index in sequence {
        let (sender, (addr, bytes)) = &calls[call_index];
        let call_result =
            executor.call_raw_committing(*sender, *addr, bytes.clone(), U256::ZERO)?;
        if call_result.reverted && failed_case.fail_on_revert {
            // Candidate sequence fails test.
            // We don't have to apply remaining calls to check sequence.
            sequence_failed = true;
            break;
        }
    }
    // Return without checking the invariant if we already have failing sequence.
    if sequence_failed {
        return Ok(false);
    };

    // Check the invariant for candidate sequence.
    // If sequence fails then we can continue with shrinking - the removed call does not affect
    // failure.
    //
    // If sequence doesn't fail then we have to restore last removed call and continue with next
    // call - removed call is a required step for reproducing the failure.
    let mut call_result =
        executor.call_raw(CALLER, failed_case.addr, failed_case.func.clone(), U256::ZERO)?;
    Ok(executor.is_raw_call_success(
        failed_case.addr,
        Cow::Owned(call_result.state_changeset.take().unwrap()),
        &call_result,
        false,
    ))
}
