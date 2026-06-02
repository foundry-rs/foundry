use crate::executors::{Executor, RawCallResult};
use alloy_primitives::U256;
use eyre::{Result, eyre};
use foundry_evm_core::{FoundryBlock, evm::FoundryEvmNetwork};
use foundry_evm_fuzz::BasicTxDetails;
use revm::context::Block;
use std::ops::ControlFlow;

/// Executes a fuzz call and returns the result.
///
/// Applies any block timestamp (warp) and block number (roll) adjustments before the call.
pub(crate) fn execute_tx<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    tx: &BasicTxDetails,
) -> Result<RawCallResult<FEN>> {
    let warp = tx.warp.unwrap_or_default();
    let roll = tx.roll.unwrap_or_default();

    if warp > 0 || roll > 0 {
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

    let requested_value = tx.call_details.value.unwrap_or(U256::ZERO);
    let sender_balance = executor.get_balance(tx.sender)?;
    let value = requested_value.min(sender_balance);
    executor
        .call_raw(tx.sender, tx.call_details.target, tx.call_details.calldata.clone(), value)
        .map_err(|e| eyre!(format!("Could not make raw evm call: {e}")))
}

/// Replays one transaction against `executor`.
///
/// `on_call` may stop early; otherwise successful calls are committed when `commit_state` is true.
pub(crate) fn replay_tx<FEN, T, F>(
    executor: &mut Executor<FEN>,
    tx: &BasicTxDetails,
    commit_state: bool,
    mut on_call: F,
) -> Result<Option<T>>
where
    FEN: FoundryEvmNetwork,
    F: FnMut(&Executor<FEN>, RawCallResult<FEN>) -> Result<ControlFlow<T, RawCallResult<FEN>>>,
{
    let call_result = execute_tx(executor, tx)?;
    match on_call(executor, call_result)? {
        ControlFlow::Break(val) => Ok(Some(val)),
        ControlFlow::Continue(mut call_result) => {
            commit_call(executor, &mut call_result, commit_state);
            Ok(None)
        }
    }
}

/// Replays `sequence` (indices into `calls`) against `executor`.
///
/// When `accumulate_warp_roll` is set, warp/roll from skipped calls is folded into the next
/// included call. `on_call` may stop early; otherwise successful calls are committed when
/// `commit_state` is true.
pub(crate) fn replay_sequence<FEN, T, F>(
    executor: &mut Executor<FEN>,
    calls: &[BasicTxDetails],
    sequence: &[usize],
    accumulate_warp_roll: bool,
    commit_state: bool,
    mut on_call: F,
) -> eyre::Result<Option<T>>
where
    FEN: FoundryEvmNetwork,
    F: FnMut(
        &Executor<FEN>,
        usize,
        RawCallResult<FEN>,
    ) -> eyre::Result<ControlFlow<T, RawCallResult<FEN>>>,
{
    // Fast path: no warp/roll accumulation -> iterate only kept indices (O(k)) and pass
    // `&calls[idx]` directly to skip the per-call `BasicTxDetails` clone.
    if !accumulate_warp_roll {
        for &idx in sequence {
            let call_result = execute_tx(executor, &calls[idx])?;
            match on_call(executor, idx, call_result)? {
                ControlFlow::Break(val) => return Ok(Some(val)),
                ControlFlow::Continue(mut call_result) => {
                    commit_call(executor, &mut call_result, commit_state);
                }
            }
        }
        return Ok(None);
    }

    // Accumulating path: must scan the full `calls` so warp/roll from skipped txs lands on
    // the next kept tx as a concrete delta.
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

        let executed = apply_warp_roll(tx, accumulated_warp, accumulated_roll);
        let call_result = execute_tx(executor, &executed)?;

        match on_call(executor, idx, call_result)? {
            ControlFlow::Break(val) => return Ok(Some(val)),
            ControlFlow::Continue(mut call_result) => {
                commit_call(executor, &mut call_result, commit_state);
            }
        }

        accumulated_warp = U256::ZERO;
        accumulated_roll = U256::ZERO;
    }

    Ok(None)
}

fn commit_call<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    call_result: &mut RawCallResult<FEN>,
    commit_state: bool,
) {
    if commit_state && !call_result.reverted {
        executor.commit(call_result);
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
