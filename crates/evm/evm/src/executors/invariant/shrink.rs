use crate::executors::{
    Executor,
    invariant::{
        call_after_invariant_function, call_invariant_function, error::FailedInvariantCaseData,
    },
};
use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::constants::MAGIC_ASSUME;
use foundry_evm_fuzz::BasicTxDetails;
use indicatif::ProgressBar;
use proptest::bits::{BitSetLike, VarBitSet};

/// Shrinker for a call sequence failure.
/// Iterates sequence call sequence top down and removes calls one by one.
/// If the failure is still reproducible with removed call then moves to the next one.
/// If the failure is not reproducible then restore removed call and moves to next one.
#[derive(Debug)]
struct CallSequenceShrinker {
    /// Length of call sequence to be shrunk.
    call_sequence_len: usize,
    /// Call ids contained in current shrunk sequence.
    included_calls: VarBitSet,
}

impl CallSequenceShrinker {
    fn new(call_sequence_len: usize) -> Self {
        Self { call_sequence_len, included_calls: VarBitSet::saturated(call_sequence_len) }
    }

    /// Return candidate shrink sequence to be tested, by removing ids from original sequence.
    fn current(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.call_sequence_len).filter(|&call_id| self.included_calls.test(call_id))
    }
}

/// Shrinks the failure case to its smallest sequence of calls.
///
/// The shrunk call sequence always respect the order failure is reproduced as it is tested
/// top-down.
pub(crate) fn shrink_sequence(
    failed_case: &FailedInvariantCaseData,
    calls: &[BasicTxDetails],
    executor: &Executor,
    call_after_invariant: bool,
    progress: Option<&ProgressBar>,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking sequence of {} calls.", calls.len());

    // Reset run count and display shrinking message.
    if let Some(progress) = progress {
        progress.set_length(failed_case.shrink_run_limit as usize as u64);
        progress.reset();
        progress.set_message(" Shrink");
    }

    // Special case test: the invariant is *unsatisfiable* - it took 0 calls to
    // break the invariant -- consider emitting a warning.
    let (_, success) =
        call_invariant_function(executor, failed_case.addr, failed_case.calldata.clone())?;
    if !success {
        return Ok(vec![]);
    }

    let mut call_idx = 0;

    let mut shrinker = CallSequenceShrinker::new(calls.len());
    for _ in 0..failed_case.shrink_run_limit {
        // Remove call at current index.
        shrinker.included_calls.clear(call_idx);

        match check_sequence(
            executor.clone(),
            calls,
            shrinker.current().collect(),
            failed_case.addr,
            failed_case.calldata.clone(),
            failed_case.fail_on_revert,
            call_after_invariant,
        ) {
            // If candidate sequence still fails, shrink until shortest possible.
            Ok((false, _)) if shrinker.included_calls.count() == 1 => break,
            // Restore last removed call as it caused sequence to pass invariant.
            Ok((true, _)) => shrinker.included_calls.set(call_idx),
            _ => {}
        }

        if let Some(progress) = progress {
            progress.inc(1);
        }

        // Restart from first call once we reach the end of sequence.
        if call_idx + 1 == shrinker.call_sequence_len {
            call_idx = 0;
        } else {
            call_idx += 1;
        };
    }

    Ok(shrinker.current().map(|idx| &calls[idx]).cloned().collect())
}

/// Checks if the given call sequence breaks the invariant.
///
/// Used in shrinking phase for checking candidate sequences and in replay failures phase to test
/// persisted failures.
/// Returns the result of invariant check (and afterInvariant call if needed) and if sequence was
/// entirely applied.
pub fn check_sequence(
    mut executor: Executor,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
    fail_on_revert: bool,
    call_after_invariant: bool,
) -> eyre::Result<(bool, bool)> {
    // Apply the call sequence.
    for call_index in sequence {
        let tx = &calls[call_index];
        let call_result = executor.transact_raw(
            tx.sender,
            tx.call_details.target,
            tx.call_details.calldata.clone(),
            U256::ZERO,
        )?;
        // Ignore calls reverted with `MAGIC_ASSUME`. This is needed to handle failed scenarios that
        // are replayed with a modified version of test driver (that use new `vm.assume`
        // cheatcodes).
        if call_result.reverted && fail_on_revert && call_result.result.as_ref() != MAGIC_ASSUME {
            // Candidate sequence fails test.
            // We don't have to apply remaining calls to check sequence.
            return Ok((false, false));
        }
    }

    // Check the invariant for call sequence.
    let (_, mut success) = call_invariant_function(&executor, test_address, calldata)?;
    // Check after invariant result if invariant is success and `afterInvariant` function is
    // declared.
    if success && call_after_invariant {
        (_, success) = call_after_invariant_function(&executor, test_address)?;
    }

    Ok((success, true))
}
