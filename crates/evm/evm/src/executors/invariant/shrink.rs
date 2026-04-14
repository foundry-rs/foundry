use crate::executors::{
    EarlyExit, EvmError, Executor, RawCallResult,
    invariant::{call_after_invariant_function, call_invariant_function, execute_tx},
};
use alloy_primitives::{Address, Bytes, I256, U256};
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    FoundryBlock, constants::MAGIC_ASSUME, decode::RevertDecoder, evm::FoundryEvmNetwork,
};
use foundry_evm_fuzz::{BasicTxDetails, invariant::InvariantContract};
use indicatif::ProgressBar;
use proptest::bits::{BitSetLike, VarBitSet};
use revm::context::Block;

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

    /// Advance to the next call index, wrapping around to 0 at the end.
    const fn next_index(&self, call_idx: usize) -> usize {
        if call_idx + 1 == self.call_sequence_len { 0 } else { call_idx + 1 }
    }
}

/// Resets the progress bar for shrinking.
fn reset_shrink_progress(config: &InvariantConfig, progress: Option<&ProgressBar>) {
    if let Some(progress) = progress {
        progress.set_length(config.shrink_run_limit as u64);
        progress.reset();
        progress.set_message(" Shrink");
    }
}

/// Applies accumulated warp/roll to a call, returning a modified copy.
fn apply_warp_roll(call: &BasicTxDetails, warp: U256, roll: U256) -> BasicTxDetails {
    let mut result = call.clone();
    if warp > U256::ZERO {
        result.warp = Some(warp);
    }
    if roll > U256::ZERO {
        result.roll = Some(roll);
    }
    result
}

/// Applies warp/roll adjustments directly to the executor's environment.
fn apply_warp_roll_to_env<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    warp: U256,
    roll: U256,
) {
    if warp > U256::ZERO || roll > U256::ZERO {
        let ts = executor.evm_env().block_env.timestamp();
        let num = executor.evm_env().block_env.number();
        executor.evm_env_mut().block_env.set_timestamp(ts + warp);
        executor.evm_env_mut().block_env.set_number(num + roll);

        let block_env = executor.evm_env().block_env.clone();
        if let Some(cheatcodes) = executor.inspector_mut().cheatcodes.as_mut() {
            if let Some(block) = cheatcodes.block.as_mut() {
                let bts = block.timestamp();
                let bnum = block.number();
                block.set_timestamp(bts + warp);
                block.set_number(bnum + roll);
            } else {
                cheatcodes.block = Some(block_env);
            }
        }
    }
}

/// Builds the final shrunk sequence from the shrinker state.
///
/// When `accumulate_warp_roll` is enabled, warp/roll from removed calls is folded into the next
/// kept call so the final sequence remains reproducible.
fn build_shrunk_sequence(
    calls: &[BasicTxDetails],
    shrinker: &CallSequenceShrinker,
    accumulate_warp_roll: bool,
) -> Vec<BasicTxDetails> {
    if !accumulate_warp_roll {
        return shrinker.current().map(|idx| calls[idx].clone()).collect();
    }

    let mut result = Vec::new();
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;

    for (idx, call) in calls.iter().enumerate() {
        accumulated_warp += call.warp.unwrap_or(U256::ZERO);
        accumulated_roll += call.roll.unwrap_or(U256::ZERO);

        if shrinker.included_calls.test(idx) {
            result.push(apply_warp_roll(call, accumulated_warp, accumulated_roll));
            accumulated_warp = U256::ZERO;
            accumulated_roll = U256::ZERO;
        }
    }

    result
}

pub(crate) fn shrink_sequence<FEN: FoundryEvmNetwork>(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    calls: &[BasicTxDetails],
    executor: &Executor<FEN>,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking sequence of {} calls.", calls.len());

    reset_shrink_progress(config, progress);

    let target_address = invariant_contract.address;
    let calldata: Bytes = invariant_contract.invariant_function.selector().to_vec().into();
    // Special case test: the invariant is *unsatisfiable* - it took 0 calls to
    // break the invariant -- consider emitting a warning.
    let (_, success) = call_invariant_function(executor, target_address, calldata.clone())?;
    if !success {
        return Ok(vec![]);
    }

    let accumulate_warp_roll = config.has_delay();
    let mut call_idx = 0;
    let mut shrinker = CallSequenceShrinker::new(calls.len());

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        shrinker.included_calls.clear(call_idx);

        match check_sequence(
            executor.clone(),
            calls,
            shrinker.current().collect(),
            target_address,
            calldata.clone(),
            CheckSequenceOptions {
                accumulate_warp_roll,
                fail_on_revert: config.fail_on_revert,
                call_after_invariant: invariant_contract.call_after_invariant,
                rd: None,
            },
        ) {
            // If candidate sequence still fails, shrink until shortest possible.
            Ok((false, _, _)) if shrinker.included_calls.count() == 1 => break,
            // Restore last removed call as it caused sequence to pass invariant.
            Ok((true, _, _)) => shrinker.included_calls.set(call_idx),
            _ => {}
        }

        if let Some(progress) = progress {
            progress.inc(1);
        }

        call_idx = shrinker.next_index(call_idx);
    }

    Ok(build_shrunk_sequence(calls, &shrinker, accumulate_warp_roll))
}

/// Checks if the given call sequence breaks the invariant.
///
/// Used in shrinking phase for checking candidate sequences and in replay failures phase to test
/// persisted failures.
/// Returns the result of invariant check (and afterInvariant call if needed) and if sequence was
/// entirely applied.
///
/// When `options.accumulate_warp_roll` is enabled, warp/roll from removed calls is folded into the
/// next kept call so the candidate sequence stays representable as a concrete counterexample.
pub fn check_sequence<FEN: FoundryEvmNetwork>(
    executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
    options: CheckSequenceOptions<'_>,
) -> eyre::Result<(bool, bool, Option<String>)> {
    if options.accumulate_warp_roll {
        check_sequence_with_accumulation(executor, calls, sequence, test_address, calldata, options)
    } else {
        check_sequence_simple(executor, calls, sequence, test_address, calldata, options)
    }
}

fn check_sequence_simple<FEN: FoundryEvmNetwork>(
    mut executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
    options: CheckSequenceOptions<'_>,
) -> eyre::Result<(bool, bool, Option<String>)> {
    // Apply the call sequence.
    for call_index in sequence {
        let tx = &calls[call_index];
        let mut call_result = execute_tx(&mut executor, tx)?;
        executor.commit(&mut call_result);
        // Ignore calls reverted with `MAGIC_ASSUME`. This is needed to handle failed scenarios that
        // are replayed with a modified version of test driver (that use new `vm.assume`
        // cheatcodes).
        if call_result.reverted
            && options.fail_on_revert
            && call_result.result.as_ref() != MAGIC_ASSUME
        {
            // Candidate sequence fails test.
            // We don't have to apply remaining calls to check sequence.
            return Ok((false, false, call_failure_reason(call_result, options.rd)));
        }
    }

    finish_sequence_check(&executor, test_address, calldata, &options)
}

fn check_sequence_with_accumulation<FEN: FoundryEvmNetwork>(
    mut executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
    options: CheckSequenceOptions<'_>,
) -> eyre::Result<(bool, bool, Option<String>)> {
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;
    let mut seq_iter = sequence.iter().peekable();

    for (idx, tx) in calls.iter().enumerate() {
        accumulated_warp += tx.warp.unwrap_or(U256::ZERO);
        accumulated_roll += tx.roll.unwrap_or(U256::ZERO);

        if seq_iter.peek() != Some(&&idx) {
            continue;
        }

        seq_iter.next();

        let tx_with_accumulated = apply_warp_roll(tx, accumulated_warp, accumulated_roll);
        let mut call_result = execute_tx(&mut executor, &tx_with_accumulated)?;

        if call_result.reverted {
            if options.fail_on_revert && call_result.result.as_ref() != MAGIC_ASSUME {
                return Ok((false, false, call_failure_reason(call_result, options.rd)));
            }
        } else {
            executor.commit(&mut call_result);
        }

        accumulated_warp = U256::ZERO;
        accumulated_roll = U256::ZERO;
    }

    // Unlike optimization mode we intentionally do not apply trailing warp/roll before the
    // invariant call: those delays would not be representable in the final shrunk sequence.
    finish_sequence_check(&executor, test_address, calldata, &options)
}

fn finish_sequence_check<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    test_address: Address,
    calldata: Bytes,
    options: &CheckSequenceOptions<'_>,
) -> eyre::Result<(bool, bool, Option<String>)> {
    let (invariant_result, mut success) =
        call_invariant_function(executor, test_address, calldata)?;
    if !success {
        return Ok((false, true, call_failure_reason(invariant_result, options.rd)));
    }

    // Check after invariant result if invariant is success and `afterInvariant` function is
    // declared.
    if success && options.call_after_invariant {
        let (after_invariant_result, after_invariant_success) =
            call_after_invariant_function(executor, test_address)?;
        success = after_invariant_success;
        if !success {
            return Ok((false, true, call_failure_reason(after_invariant_result, options.rd)));
        }
    }

    Ok((success, true, None))
}

pub struct CheckSequenceOptions<'a> {
    pub accumulate_warp_roll: bool,
    pub fail_on_revert: bool,
    pub call_after_invariant: bool,
    pub rd: Option<&'a RevertDecoder>,
}

fn call_failure_reason<FEN: FoundryEvmNetwork>(
    call_result: RawCallResult<FEN>,
    rd: Option<&RevertDecoder>,
) -> Option<String> {
    match call_result.into_evm_error(rd) {
        EvmError::Execution(err) => Some(err.reason),
        _ => None,
    }
}

/// Shrinks a call sequence to the shortest sequence that still produces the target optimization
/// value. This is specifically for optimization mode where we want to find the minimal sequence
/// that achieves the maximum value.
///
/// Unlike `shrink_sequence` (for check mode), this function:
/// - Accumulates warp/roll values from removed calls into the next kept call
/// - Checks for target value equality rather than invariant failure
pub(crate) fn shrink_sequence_value<FEN: FoundryEvmNetwork>(
    config: &InvariantConfig,
    invariant_contract: &InvariantContract<'_>,
    calls: &[BasicTxDetails],
    executor: &Executor<FEN>,
    target_value: I256,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> eyre::Result<Vec<BasicTxDetails>> {
    trace!(target: "forge::test", "Shrinking optimization sequence of {} calls for target value {}.", calls.len(), target_value);

    reset_shrink_progress(config, progress);

    let target_address = invariant_contract.address;
    let calldata: Bytes = invariant_contract.invariant_function.selector().to_vec().into();

    // Special case: check if target value is achieved with 0 calls.
    if check_sequence_value(executor.clone(), calls, vec![], target_address, calldata.clone())?
        == Some(target_value)
    {
        return Ok(vec![]);
    }

    let mut call_idx = 0;
    let mut shrinker = CallSequenceShrinker::new(calls.len());

    for _ in 0..config.shrink_run_limit {
        if early_exit.should_stop() {
            break;
        }

        shrinker.included_calls.clear(call_idx);

        let keeps_target = check_sequence_value(
            executor.clone(),
            calls,
            shrinker.current().collect(),
            target_address,
            calldata.clone(),
        )? == Some(target_value);

        if keeps_target {
            if shrinker.included_calls.count() == 1 {
                break;
            }
        } else {
            shrinker.included_calls.set(call_idx);
        }

        if let Some(progress) = progress {
            progress.inc(1);
        }

        call_idx = shrinker.next_index(call_idx);
    }

    Ok(build_shrunk_sequence(calls, &shrinker, true))
}

/// Executes a call sequence and returns the optimization value (int256) from the invariant
/// function. Used during shrinking for optimization mode.
///
/// Returns `None` if the invariant call fails or doesn't return a valid int256.
/// Unlike `check_sequence`, this applies warp/roll from ALL calls (including removed ones).
pub fn check_sequence_value<FEN: FoundryEvmNetwork>(
    mut executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: Vec<usize>,
    test_address: Address,
    calldata: Bytes,
) -> eyre::Result<Option<I256>> {
    let mut accumulated_warp = U256::ZERO;
    let mut accumulated_roll = U256::ZERO;
    let mut seq_iter = sequence.iter().peekable();

    for (idx, tx) in calls.iter().enumerate() {
        accumulated_warp += tx.warp.unwrap_or(U256::ZERO);
        accumulated_roll += tx.roll.unwrap_or(U256::ZERO);

        if seq_iter.peek() == Some(&&idx) {
            seq_iter.next();

            let tx_with_accumulated = apply_warp_roll(tx, accumulated_warp, accumulated_roll);
            let mut call_result = execute_tx(&mut executor, &tx_with_accumulated)?;

            if !call_result.reverted {
                executor.commit(&mut call_result);
            }

            accumulated_warp = U256::ZERO;
            accumulated_roll = U256::ZERO;
        }
    }

    // Apply any remaining accumulated warp/roll before calling invariant.
    apply_warp_roll_to_env(&mut executor, accumulated_warp, accumulated_roll);

    let (inv_result, success) = call_invariant_function(&executor, test_address, calldata)?;

    if success
        && inv_result.result.len() >= 32
        && let Some(value) = I256::try_from_be_slice(&inv_result.result[..32])
    {
        return Ok(Some(value));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{CallSequenceShrinker, build_shrunk_sequence};
    use alloy_primitives::{Address, Bytes, U256};
    use foundry_evm_fuzz::{BasicTxDetails, CallDetails};
    use proptest::bits::BitSetLike;

    fn tx(warp: Option<u64>, roll: Option<u64>) -> BasicTxDetails {
        BasicTxDetails {
            warp: warp.map(U256::from),
            roll: roll.map(U256::from),
            sender: Address::ZERO,
            call_details: CallDetails { target: Address::ZERO, calldata: Bytes::new() },
        }
    }

    #[test]
    fn build_shrunk_sequence_accumulates_removed_delay_into_next_kept_call() {
        let calls = vec![tx(Some(3), Some(5)), tx(Some(7), Some(11)), tx(Some(13), Some(17))];
        let mut shrinker = CallSequenceShrinker::new(calls.len());
        shrinker.included_calls.clear(0);

        let shrunk = build_shrunk_sequence(&calls, &shrinker, true);

        assert_eq!(shrunk.len(), 2);
        assert_eq!(shrunk[0].warp, Some(U256::from(10)));
        assert_eq!(shrunk[0].roll, Some(U256::from(16)));
        assert_eq!(shrunk[1].warp, Some(U256::from(13)));
        assert_eq!(shrunk[1].roll, Some(U256::from(17)));
    }

    #[test]
    fn build_shrunk_sequence_does_not_move_trailing_delay_backward() {
        let calls = vec![tx(Some(3), Some(5)), tx(Some(7), Some(11))];
        let mut shrinker = CallSequenceShrinker::new(calls.len());
        shrinker.included_calls.clear(1);

        let shrunk = build_shrunk_sequence(&calls, &shrinker, true);

        assert_eq!(shrunk.len(), 1);
        assert_eq!(shrunk[0].warp, Some(U256::from(3)));
        assert_eq!(shrunk[0].roll, Some(U256::from(5)));
    }
}
