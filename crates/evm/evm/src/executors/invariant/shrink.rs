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

pub(crate) fn shrink_sequence(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    calls: &[BasicTxDetails],
    executor: &Executor,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking sequence of {} calls.", calls.len());

    // Reset run count and display shrinking message.
    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        progress.set_message(" Shrink");
    }

    let target_address = invariant_contract.address;
    let calldata: Bytes = invariant_contract.invariant_function.selector().to_vec().into();
    // Special case test: the invariant is *unsatisfiable* - it took 0 calls to
    // break the invariant -- consider emitting a warning.
    let (_, success) = call_invariant_function(executor, target_address, calldata.clone())?;
    if !success {
        return Ok(vec![]);
    }

    let mut call_idx = 0;

    let mut shrinker = CallSequenceShrinker::new(calls.len());
    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        // Remove call at current index.
        shrinker.included_calls.clear(call_idx);

        match check_sequence(
            executor.clone(),
            calls,
            shrinker.current().collect(),
            target_address,
            calldata.clone(),
            config.fail_on_revert,
            invariant_contract.call_after_invariant,
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

/// Shrinks a call sequence to the shortest sequence that still produces the target optimization
/// value. This is specifically for optimization mode where we want to find the minimal sequence
/// that achieves the maximum value.
///
/// Unlike `shrink_sequence` (for check mode), this function:
/// - Accumulates warp/roll values from removed calls into the next kept call
/// - Checks for target value equality rather than invariant failure
pub(crate) fn shrink_sequence_value(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    calls: &[BasicTxDetails],
    executor: &Executor,
    target_value: I256,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking optimization sequence of {} calls for target value {}.", calls.len(), target_value);

    // Reset run count and display shrinking message.
    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        progress.set_message(" Shrink");
    }

    let target_address = invariant_contract.address;
    let calldata: Bytes = invariant_contract.invariant_function.selector().to_vec().into();

    // Special case: check if target value is achieved with 0 calls
    if let Some(value) = check_sequence_value(
        executor.clone(),
        calls,
        vec![],
        target_address,
        calldata.clone(),
    )?
        && value == target_value
    {
        return Ok(vec![]);
    }

    let mut call_idx = 0;
    let mut shrinker = CallSequenceShrinker::new(calls.len());

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        // Remove call at current index.
        shrinker.included_calls.clear(call_idx);

        let current_indices: Vec<usize> = shrinker.current().collect();
        if let Some(value) = check_sequence_value(
            executor.clone(),
            calls,
            current_indices,
            target_address,
            calldata.clone(),
        )? {
            if value == target_value {
                // Sequence still achieves target, check if we're at minimum
                if shrinker.included_calls.count() == 1 {
                    break;
                }
            } else {
                // Target not achieved, restore the call
                shrinker.included_calls.set(call_idx);
            }
        } else {
            // Execution failed, restore the call
            shrinker.included_calls.set(call_idx);
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

    // Build the final shrunk sequence, accumulating warp/roll from removed calls.
    let mut result = Vec::new();
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;

    for (idx, call) in calls.iter().enumerate() {
        // Always accumulate warp/roll from this call
        accumulated_warp += call.warp.unwrap_or(U256::ZERO);
        accumulated_roll += call.roll.unwrap_or(U256::ZERO);

        if shrinker.included_calls.test(idx) {
            // This call is kept - apply accumulated warp/roll to it
            let mut kept_call = call.clone();
            if accumulated_warp > U256::ZERO {
                kept_call.warp = Some(accumulated_warp);
            }
            if accumulated_roll > U256::ZERO {
                kept_call.roll = Some(accumulated_roll);
            }
            result.push(kept_call);
            // Reset accumulators after applying
            accumulated_warp = U256::ZERO;
            accumulated_roll = U256::ZERO;
        }
    }

    Ok(result)
}

/// Executes a call sequence and returns the optimization value (int256) from the invariant
/// function. This is used during shrinking for optimization mode.
///
/// Returns None if the invariant call fails or doesn't return a valid int256.
/// Unlike `check_sequence`, this function:
/// - Applies warp/roll from ALL calls (including removed ones via accumulation in caller)
/// - Returns the optimization value rather than success/failure
pub fn check_sequence_value(
    mut executor: Executor,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
) -> eyre::Result<Option<I256>> {
    // Track accumulated warp/roll from all calls up to each kept call
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;
    let mut seq_iter = sequence.iter().peekable();

    for (idx, tx) in calls.iter().enumerate() {
        // Accumulate warp/roll from this call
        accumulated_warp += tx.warp.unwrap_or(U256::ZERO);
        accumulated_roll += tx.roll.unwrap_or(U256::ZERO);

        // Check if this index is in the sequence
        if seq_iter.peek() == Some(&&idx) {
            seq_iter.next();

            // Create a modified tx with accumulated warp/roll
            let mut tx_with_accumulated = tx.clone();
            if accumulated_warp > U256::ZERO {
                tx_with_accumulated.warp = Some(accumulated_warp);
            }
            if accumulated_roll > U256::ZERO {
                tx_with_accumulated.roll = Some(accumulated_roll);
            }

            let mut call_result = execute_tx(&mut executor, &tx_with_accumulated)?;

            // Skip commits for reverted calls (but we've already applied warp/roll)
            if !call_result.reverted {
                executor.commit(&mut call_result);
            }

            // Reset accumulators after applying to a kept call
            accumulated_warp = U256::ZERO;
            accumulated_roll = U256::ZERO;
        }
    }

    // Apply any remaining accumulated warp/roll to the executor's env before calling invariant
    if accumulated_warp > U256::ZERO || accumulated_roll > U256::ZERO {
        executor.env_mut().evm_env.block_env.timestamp += accumulated_warp;
        executor.env_mut().evm_env.block_env.number += accumulated_roll;

        let block_env = executor.env().evm_env.block_env.clone();
        if let Some(cheatcodes) = executor.inspector_mut().cheatcodes.as_mut() {
            if let Some(block) = cheatcodes.block.as_mut() {
                block.timestamp += accumulated_warp;
                block.number += accumulated_roll;
            } else {
                cheatcodes.block = Some(block_env);
            }
        }
    }

    // Call the invariant function and extract the int256 value
    let (inv_result, success) = call_invariant_function(&executor, test_address, calldata)?;

    if success
        && inv_result.result.len() >= 32
        && let Some(value) = I256::try_from_be_slice(&inv_result.result[..32])
    {
        return Ok(Some(value));
    }

    Ok(None)
}
