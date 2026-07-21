//! Worker-local call execution policy shared by stateless and invariant fuzz campaigns.

use super::{Executor, RawCallResult};
use alloy_primitives::U256;
use eyre::{Result, eyre};
use foundry_evm_core::{
    FoundryBlock,
    constants::MAGIC_ASSUME,
    evm::{BlockEnvFor, FoundryEvmNetwork},
};
use foundry_evm_fuzz::BasicTxDetails;
use revm::context::Block;

/// The small set of execution policies which differ between fuzzing modes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FuzzCampaignMode {
    /// Stateless calls never mutate the worker executor.
    Stateless,
    /// Accepted invariant calls mutate it and predicates run at the configured cadence.
    Invariant { check_interval: u32, optimization: bool },
}

/// Classification produced by the common call loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CampaignCallKind {
    Accepted,
    AssumptionRejected,
}

pub(crate) enum CampaignEvent<'a, FEN: FoundryEvmNetwork> {
    Feedback(&'a mut RawCallResult<FEN>),
    Check { result: &'a mut Option<RawCallResult<FEN>>, kind: CampaignCallKind, should_check: bool },
    Advance,
    Next { discarded: bool, depth: u32 },
    PostCheck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CampaignControl {
    Continue,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CampaignSequenceOutcome {
    Complete,
    Cancelled,
    Stopped,
}

/// Concrete worker-local campaign execution policy.
pub(crate) struct FuzzCampaign {
    mode: FuzzCampaignMode,
}

impl FuzzCampaign {
    pub(crate) const fn new(mode: FuzzCampaignMode) -> Self {
        Self { mode }
    }

    /// Drives one concrete sequence, including execution, classification, and lifecycle events.
    pub(crate) fn run_sequence<S, FEN, Parts, Depth, Stop, Event>(
        &self,
        state: &mut S,
        run_depth: u32,
        mut parts: Parts,
        mut depth: Depth,
        mut should_stop: Stop,
        mut on_event: Event,
    ) -> Result<CampaignSequenceOutcome>
    where
        FEN: FoundryEvmNetwork,
        Parts: for<'a> FnMut(&'a mut S) -> (&'a mut Executor<FEN>, &'a mut BasicTxDetails),
        Depth: FnMut(&S) -> u32,
        Stop: FnMut(&S) -> bool,
        Event: for<'a> FnMut(&mut S, CampaignEvent<'a, FEN>) -> Result<CampaignControl>,
    {
        let outcome = loop {
            if depth(state) >= run_depth {
                break CampaignSequenceOutcome::Complete;
            }
            if should_stop(state) {
                return Ok(CampaignSequenceOutcome::Cancelled);
            }
            let current_depth = depth(state);
            let last_call = current_depth == run_depth - 1;
            let (mut result, block_snapshot) = {
                let (executor, tx) = parts(state);
                let snapshot = matches!(self.mode, FuzzCampaignMode::Invariant { .. })
                    .then(|| BlockSnapshot::new(executor));
                let result = match self.mode {
                    FuzzCampaignMode::Stateless => executor.call_raw(
                        tx.sender,
                        tx.call_details.target,
                        tx.call_details.calldata.clone(),
                        tx.call_details.value.unwrap_or_default(),
                    )?,
                    FuzzCampaignMode::Invariant { .. } => execute_invariant_tx(executor, tx)?,
                };
                (result, snapshot)
            };
            if result.execution_cancelled {
                if let Some(snapshot) = block_snapshot {
                    let (executor, _) = parts(state);
                    snapshot.restore(executor);
                }
                return Ok(CampaignSequenceOutcome::Cancelled);
            }
            on_event(state, CampaignEvent::Feedback(&mut result))?;
            let discarded = result.result.as_ref() == MAGIC_ASSUME;
            let kind = if discarded {
                if let Some(snapshot) = block_snapshot {
                    let (executor, _) = parts(state);
                    snapshot.restore(executor);
                }
                CampaignCallKind::AssumptionRejected
            } else {
                if matches!(self.mode, FuzzCampaignMode::Invariant { .. }) {
                    let (executor, _) = parts(state);
                    executor.commit(&mut result);
                }
                CampaignCallKind::Accepted
            };
            let should_check = match self.mode {
                FuzzCampaignMode::Stateless => true,
                FuzzCampaignMode::Invariant { optimization: true, .. } => true,
                FuzzCampaignMode::Invariant { check_interval: 0, .. } => last_call,
                FuzzCampaignMode::Invariant { check_interval, .. } => {
                    check_interval == 1
                        || (current_depth + 1).is_multiple_of(check_interval)
                        || last_call
                }
            };
            let mut result = Some(result);
            if on_event(state, CampaignEvent::Check { result: &mut result, kind, should_check })?
                == CampaignControl::Stop
            {
                break CampaignSequenceOutcome::Stopped;
            }
            if !discarded {
                on_event(state, CampaignEvent::Advance)?;
            }
            let next_depth = depth(state);
            on_event(state, CampaignEvent::Next { discarded, depth: next_depth })?;
        };
        on_event(state, CampaignEvent::PostCheck)?;
        Ok(outcome)
    }
}

struct BlockSnapshot<FEN: FoundryEvmNetwork> {
    env: BlockEnvFor<FEN>,
    cheatcode: Option<BlockEnvFor<FEN>>,
}

impl<FEN: FoundryEvmNetwork> BlockSnapshot<FEN> {
    fn new(executor: &Executor<FEN>) -> Self {
        Self {
            env: executor.evm_env().block_env.clone(),
            cheatcode: executor.inspector().cheatcodes.as_ref().and_then(|c| c.block.clone()),
        }
    }

    fn restore(self, executor: &mut Executor<FEN>) {
        executor.evm_env_mut().block_env = self.env;
        if let Some(cheatcodes) = executor.inspector_mut().cheatcodes.as_mut() {
            cheatcodes.block = self.cheatcode;
        }
    }
}

pub(super) fn execute_invariant_tx<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    tx: &mut BasicTxDetails,
) -> Result<RawCallResult<FEN>> {
    let warp = tx.warp.unwrap_or_default();
    let roll = tx.roll.unwrap_or_default();
    if warp > 0 || roll > 0 {
        let needs_cheatcode_block = executor
            .inspector()
            .cheatcodes
            .as_ref()
            .is_some_and(|cheatcodes| cheatcodes.block.is_none());
        let block_env = {
            let block_env = &mut executor.evm_env_mut().block_env;
            block_env.set_timestamp(block_env.timestamp() + warp);
            block_env.set_number(block_env.number() + roll);
            needs_cheatcode_block.then(|| block_env.clone())
        };
        if let Some(cheatcodes) = executor.inspector_mut().cheatcodes.as_mut() {
            if let Some(block) = cheatcodes.block.as_mut() {
                block.set_timestamp(block.timestamp() + warp);
                block.set_number(block.number() + roll);
            } else {
                cheatcodes.block = Some(block_env.unwrap());
            }
        }
    }
    let value = match tx.call_details.value {
        Some(requested) if !requested.is_zero() => requested.min(executor.get_balance(tx.sender)?),
        _ => U256::ZERO,
    };
    // Persist exactly what was sent so replay, shrinking, and corpus entries do not claim an
    // unavailable value was executed.
    tx.call_details.value = (!value.is_zero()).then_some(value);
    executor
        .call_raw(tx.sender, tx.call_details.target, tx.call_details.calldata.clone(), value)
        .map_err(|error| eyre!("Could not make raw evm call: {error}"))
}
