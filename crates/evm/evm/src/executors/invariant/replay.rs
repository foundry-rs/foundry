use super::{call_after_invariant_function, call_invariant_function};
use crate::executors::{EarlyExit, Executor, invariant::shrink::shrink_sequence};
use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::{Log, U256, map::HashMap};
use eyre::Result;
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_config::InvariantConfig;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{BaseCounterExample, BasicTxDetails, invariant::InvariantContract};
use foundry_evm_traces::{TraceKind, TraceMode, Traces, load_contracts};
use indicatif::ProgressBar;
use parking_lot::RwLock;
use std::sync::Arc;

/// Replays a call sequence for collecting logs and traces.
/// Returns counterexample to be used when the call sequence is a failed scenario.
#[expect(clippy::too_many_arguments)]
pub fn replay_run(
    invariant_contract: &InvariantContract<'_>,
    mut executor: Executor,
    known_contracts: &ContractsByArtifact,
    mut ided_contracts: ContractsByAddress,
    logs: &mut Vec<Log>,
    traces: &mut Traces,
    line_coverage: &mut Option<HitMaps>,
    deprecated_cheatcodes: &mut HashMap<&'static str, Option<&'static str>>,
    inputs: &[BasicTxDetails],
    show_solidity: bool,
) -> Result<Vec<BaseCounterExample>> {
    // We want traces for a failed case.
    if executor.inspector().tracer.is_none() {
        executor.set_tracing(TraceMode::Call);
    }

    let mut counterexample_sequence = vec![];

    // Replay each call from the sequence, collect logs, traces and coverage.
    for tx in inputs {
        let call_result = executor.transact_raw(
            tx.sender,
            tx.call_details.target,
            tx.call_details.calldata.clone(),
            U256::ZERO,
        )?;

        logs.extend(call_result.logs);
        traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));
        HitMaps::merge_opt(line_coverage, call_result.line_coverage);

        // Identify newly generated contracts, if they exist.
        ided_contracts
            .extend(load_contracts(call_result.traces.iter().map(|a| &a.arena), known_contracts));

        // Create counter example to be used in failed case.
        counterexample_sequence.push(BaseCounterExample::from_invariant_call(
            tx.sender,
            tx.call_details.target,
            &tx.call_details.calldata,
            &ided_contracts,
            call_result.traces,
            show_solidity,
        ));
    }

    // Replay invariant to collect logs and traces.
    // We do this only once at the end of the replayed sequence.
    // Checking after each call doesn't add valuable info for passing scenario
    // (invariant call result is always success) nor for failed scenarios
    // (invariant call result is always success until the last call that breaks it).
    let (invariant_result, invariant_success) = call_invariant_function(
        &executor,
        invariant_contract.address,
        invariant_contract.invariant_function.abi_encode_input(&[])?.into(),
    )?;
    traces.push((TraceKind::Execution, invariant_result.traces.clone().unwrap()));
    logs.extend(invariant_result.logs);
    deprecated_cheatcodes.extend(
        invariant_result
            .cheatcodes
            .as_ref()
            .map_or_else(Default::default, |cheats| cheats.deprecated.clone()),
    );

    // Collect after invariant logs and traces.
    if invariant_contract.call_after_invariant && invariant_success {
        let (after_invariant_result, _) =
            call_after_invariant_function(&executor, invariant_contract.address)?;
        traces.push((TraceKind::Execution, after_invariant_result.traces.clone().unwrap()));
        logs.extend(after_invariant_result.logs);
    }

    Ok(counterexample_sequence)
}

/// Replays the error case, shrinks the failing sequence and collects all necessary traces.
#[expect(clippy::too_many_arguments)]
pub fn replay_error(
    config: InvariantConfig,
    mut executor: Executor,
    calls: &[BasicTxDetails],
    inner_sequence: Option<Vec<Option<BasicTxDetails>>>,
    invariant_contract: &InvariantContract<'_>,
    known_contracts: &ContractsByArtifact,
    ided_contracts: ContractsByAddress,
    logs: &mut Vec<Log>,
    traces: &mut Traces,
    line_coverage: &mut Option<HitMaps>,
    deprecated_cheatcodes: &mut HashMap<&'static str, Option<&'static str>>,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
) -> Result<Vec<BaseCounterExample>> {
    // Shrink sequence of failed calls.
    let calls =
        shrink_sequence(&config, invariant_contract, calls, &executor, progress, early_exit)?;

    if let Some(sequence) = inner_sequence {
        set_up_inner_replay(&mut executor, &sequence);
    }

    // Replay calls to get the counterexample and to collect logs, traces and coverage.
    replay_run(
        invariant_contract,
        executor,
        known_contracts,
        ided_contracts,
        logs,
        traces,
        line_coverage,
        deprecated_cheatcodes,
        &calls,
        config.show_solidity,
    )
}

/// Sets up the calls generated by the internal fuzzer, if they exist.
fn set_up_inner_replay(executor: &mut Executor, inner_sequence: &[Option<BasicTxDetails>]) {
    if let Some(fuzzer) = &mut executor.inspector_mut().fuzzer
        && let Some(call_generator) = &mut fuzzer.call_generator
    {
        call_generator.last_sequence = Arc::new(RwLock::new(inner_sequence.to_owned()));
        call_generator.set_replay(true);
    }
}
