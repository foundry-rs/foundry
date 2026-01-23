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

/// Shrinks a call sequence to find the shortest sequence that still fails the invariant.
pub(crate) fn shrink_sequence(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    calls: &[BasicTxDetails],
    executor: &Executor,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking sequence of {} calls.", calls.len());

    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        progress.set_message(" Shrink");
    }

    let target_address = invariant_contract.address;
    let calldata: Bytes = invariant_contract.invariant_function.selector().to_vec().into();

    // Special case: the invariant is *unsatisfiable* - it took 0 calls to break it.
    let (_, success) = call_invariant_function(executor, target_address, calldata.clone())?;
    if !success {
        return Ok(vec![]);
    }

    let mut shrinker = CallSequenceShrinker::new(calls.len());
    let mut call_idx = 0;

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        shrinker.included_calls.clear(call_idx);

        let should_keep_removal = match check_sequence(
            executor.clone(),
            calls,
            shrinker.current().collect(),
            target_address,
            calldata.clone(),
            config.fail_on_revert,
            invariant_contract.call_after_invariant,
        ) {
            Ok((false, _)) => true, // Still fails, keep removal
            Ok((true, _)) => false, // Now passes, restore call
            _ => false,
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

    Ok(shrinker.current().map(|idx| &calls[idx]).cloned().collect())
}

/// Shrinks a call sequence to find the shortest sequence that produces the target optimization
/// value. Accumulates warp/roll from removed calls to maintain correct block environment.
pub(crate) fn shrink_sequence_value(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    calls: &[BasicTxDetails],
    executor: &Executor,
    target_value: I256,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        progress.set_message(" Shrink (optimization)");
    }

    let target_address = invariant_contract.address;
    let calldata: Bytes = invariant_contract.invariant_function.selector().to_vec().into();

    // Check if empty sequence already produces target value.
    let initial_value =
        check_sequence_value(executor.clone(), &[], vec![], target_address, calldata.clone())?;
    if initial_value == Some(target_value) {
        return Ok(vec![]);
    }

    // Verify the full sequence produces the target value.
    let full_seq_value = check_sequence_value(
        executor.clone(),
        calls,
        (0..calls.len()).collect(),
        target_address,
        calldata.clone(),
    )?;
    if full_seq_value != Some(target_value) {
        return Ok(calls.to_vec());
    }

    let mut shrinker = CallSequenceShrinker::new(calls.len());
    let mut call_idx = 0;

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        shrinker.included_calls.clear(call_idx);

        let value = check_sequence_value(
            executor.clone(),
            calls,
            shrinker.current().collect(),
            target_address,
            calldata.clone(),
        )?;
        let should_keep_removal = value == Some(target_value);

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

    // Build shrunk sequence, accumulating warps/rolls from removed calls.
    let kept_indices: Vec<usize> = shrinker.current().collect();
    let mut result = Vec::with_capacity(kept_indices.len());
    let mut accumulated_warp: Option<U256> = None;
    let mut accumulated_roll: Option<U256> = None;

    for (idx, call) in calls.iter().enumerate() {
        if let Some(warp) = call.warp {
            accumulated_warp = Some(accumulated_warp.unwrap_or_default() + warp);
        }
        if let Some(roll) = call.roll {
            accumulated_roll = Some(accumulated_roll.unwrap_or_default() + roll);
        }

        if kept_indices.contains(&idx) {
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
    let mut seq_iter = sequence.iter().peekable();

    // Apply warps/rolls from ALL calls but only execute calls in sequence.
    for (call_index, tx) in calls.iter().enumerate() {
        // Always apply warp/roll to maintain consistent block environment.
        if let Some(warp) = tx.warp {
            executor.env_mut().evm_env.block_env.timestamp += warp;
        }
        if let Some(roll) = tx.roll {
            executor.env_mut().evm_env.block_env.number += roll;
        }

        // Execute if this call is next in the sequence.
        if seq_iter.peek() == Some(&&call_index) {
            seq_iter.next();

            // Update inspector's block env before the call.
            let block_env = executor.env().evm_env.block_env.clone();
            executor.inspector_mut().set_block(&block_env);

            let mut call_result = executor.call_raw(
                tx.sender,
                tx.call_details.target,
                tx.call_details.calldata.clone(),
                U256::ZERO,
            )?;

            // In optimization mode, reverted calls are kept for warp/roll but don't affect state.
            if !call_result.reverted {
                executor.commit(&mut call_result);
            }
        }
    }

    let (call_result, success) = call_invariant_function(&executor, test_address, calldata)?;
    if !success {
        return Ok(None);
    }

    if call_result.result.len() >= 32 {
        Ok(I256::try_from_be_slice(&call_result.result[..32]))
    } else {
        Ok(None)
    }
}
