use crate::executors::{
    EarlyExit, Executor,
    invariant::{call_after_invariant_function, call_invariant_function, execute_tx},
};
use alloy_primitives::{Address, Bytes, I256, U256};
use foundry_config::InvariantConfig;
use foundry_evm_core::constants::MAGIC_ASSUME;
use foundry_evm_fuzz::{BasicTxDetails, invariant::InvariantContract};
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

/// Shrinks a call sequence to find the shortest sequence that reproduces the failure
/// or (for optimization mode) produces the target value.
///
/// For check mode (target_value=None): finds shortest sequence that still fails the invariant.
/// For optimization mode (target_value=Some): finds shortest sequence that produces the target value.
pub(crate) fn shrink_sequence(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    calls: &[BasicTxDetails],
    executor: &Executor,
    target_value: Option<I256>,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    let is_optimization = target_value.is_some();
    trace!(target: "forge::test", "Shrinking sequence of {} calls (optimization: {}).", calls.len(), is_optimization);

    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        progress.set_message(if is_optimization { " Shrink (optimization)" } else { " Shrink" });
    }

    let target_address = invariant_contract.address;
    let calldata: Bytes = invariant_contract.invariant_function.selector().to_vec().into();

    // Check if empty sequence already satisfies the shrink goal.
    if is_optimization {
        let initial_value =
            check_sequence_value(executor.clone(), &[], vec![], target_address, calldata.clone())?;
        if initial_value == target_value {
            return Ok(vec![]);
        }

        // Verify the full sequence produces the target value.
        // If not, we can't shrink - just return the original sequence.
        let full_seq_value = check_sequence_value(
            executor.clone(),
            calls,
            (0..calls.len()).collect(),
            target_address,
            calldata.clone(),
        )?;
        if full_seq_value != target_value {
            warn!(target: "forge::test", "Optimization shrink: full sequence of {} calls doesn't reproduce target value {:?}, got {:?}.", calls.len(), target_value, full_seq_value);
            return Ok(calls.to_vec());
        }
    } else {
        let (_, success) = call_invariant_function(executor, target_address, calldata.clone())?;
        if !success {
            return Ok(vec![]);
        }
    }

    let mut call_idx = 0;
    let mut shrinker = CallSequenceShrinker::new(calls.len());

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        shrinker.included_calls.clear(call_idx);

        let should_keep_removal = if is_optimization {
            // For optimization: check if sequence still produces target value.
            let value = check_sequence_value(
                executor.clone(),
                calls,
                shrinker.current().collect(),
                target_address,
                calldata.clone(),
            )?;
            value == target_value
        } else {
            // For check mode: check if sequence still fails.
            match check_sequence(
                executor.clone(),
                calls,
                shrinker.current().collect(),
                target_address,
                calldata.clone(),
                config.fail_on_revert,
                invariant_contract.call_after_invariant,
            ) {
                Ok((false, _)) => true,  // Still fails, keep removal
                Ok((true, _)) => false,  // Now passes, restore call
                _ => false,
            }
        };

        if should_keep_removal {
            if shrinker.included_calls.count() == 1 {
                break;
            }
        } else {
            shrinker.included_calls.set(call_idx);
        }

        if let Some(progress) = progress {
            progress.inc(1);
        }

        if call_idx + 1 == shrinker.call_sequence_len {
            call_idx = 0;
        } else {
            call_idx += 1;
        };
    }

    // Build the shrunk sequence, accumulating warps/rolls from removed calls.
    // When a call is removed, its warp/roll should be added to the next kept call
    // so that the sequence reproduces the same block environment.
    let kept_indices: Vec<usize> = shrinker.current().collect();
    let mut result = Vec::with_capacity(kept_indices.len());
    let mut accumulated_warp: Option<U256> = None;
    let mut accumulated_roll: Option<U256> = None;

    for (call_idx, call) in calls.iter().enumerate() {
        // Accumulate warp/roll from all calls (kept or removed).
        if let Some(warp) = call.warp {
            accumulated_warp = Some(accumulated_warp.unwrap_or_default() + warp);
        }
        if let Some(roll) = call.roll {
            accumulated_roll = Some(accumulated_roll.unwrap_or_default() + roll);
        }

        // If this call is kept, add it with accumulated warp/roll.
        if kept_indices.contains(&call_idx) {
            let mut kept_call = call.clone();
            kept_call.warp = accumulated_warp.take();
            kept_call.roll = accumulated_roll.take();
            result.push(kept_call);
        }
    }

    Ok(result)
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
        let mut call_result = execute_tx(&mut executor, tx)?;
        executor.commit(&mut call_result);
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

/// Executes a call sequence and returns the optimization value from the invariant function.
/// Returns None if the invariant call fails or can't be decoded.
///
/// The `sequence` parameter contains indices of calls to actually execute.
/// Warps/rolls from ALL calls are applied to maintain consistent block environment,
/// but only non-reverted calls in `sequence` affect state.
pub fn check_sequence_value(
    mut executor: Executor,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
) -> eyre::Result<Option<I256>> {
    // Convert sequence to a set for O(1) lookup.
    let sequence_set: std::collections::HashSet<usize> = sequence.iter().copied().collect();

    // Apply the call sequence, but always apply warps/rolls even for skipped calls.
    for (call_index, tx) in calls.iter().enumerate() {
        // Always apply warp/roll to maintain consistent block environment.
        if let Some(warp) = tx.warp {
            executor.env_mut().evm_env.block_env.timestamp += warp;
        }
        if let Some(roll) = tx.roll {
            executor.env_mut().evm_env.block_env.number += roll;
        }

        // Only execute calls that are in the sequence.
        if sequence_set.contains(&call_index) {
            // Update inspector's block env before the call.
            let block_env = executor.env().evm_env.block_env.clone();
            executor.inspector_mut().set_block(&block_env);

            let mut call_result = executor.call_raw(
                tx.sender,
                tx.call_details.target,
                tx.call_details.calldata.clone(),
                U256::ZERO,
            )?;

            // In optimization mode, reverted calls are kept in the sequence for their warp/roll
            // values, but we skip committing their state changes.
            if !call_result.reverted {
                executor.commit(&mut call_result);
            }
        }
    }

    // Call the optimization invariant and decode the value.
    let (call_result, success) = call_invariant_function(&executor, test_address, calldata)?;
    if !success {
        return Ok(None);
    }

    // Decode int256 from result.
    if call_result.result.len() >= 32 {
        Ok(I256::try_from_be_slice(&call_result.result[..32]))
    } else {
        Ok(None)
    }
}
