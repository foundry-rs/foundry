use super::{call_after_invariant_function, call_invariant_function, execute_tx};
use crate::executors::{
    EarlyExit, Executor,
    invariant::shrink::{reset_shrink_progress, shrink_sequence, shrink_sequence_value},
};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{I256, Log, map::HashMap};
use eyre::Result;
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_config::InvariantConfig;
use foundry_evm_core::evm::FoundryEvmNetwork;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{BaseCounterExample, BasicTxDetails, invariant::InvariantContract};
use foundry_evm_traces::{TraceKind, TraceMode, Traces, load_contracts};
use indicatif::ProgressBar;
use parking_lot::RwLock;
use std::sync::Arc;

/// Replays a call sequence for collecting logs and traces.
/// Returns counterexample to be used when the call sequence is a failed scenario.
#[expect(clippy::too_many_arguments)]
pub fn replay_run<FEN: FoundryEvmNetwork>(
    invariant_contract: &InvariantContract<'_>,
    target_invariant: &Function,
    mut executor: Executor<FEN>,
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
        let mut call_result = execute_tx(&mut executor, tx)?;
        logs.extend(call_result.logs.clone());
        traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));
        HitMaps::merge_opt(line_coverage, call_result.line_coverage.clone());

        // Commit state changes to persist across calls in the sequence.
        executor.commit(&mut call_result);

        // Identify newly generated contracts, if they exist.
        ided_contracts
            .extend(load_contracts(call_result.traces.iter().map(|a| &a.arena), known_contracts));

        // Create counter example to be used in failed case.
        counterexample_sequence.push(BaseCounterExample::from_invariant_call(
            tx,
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
        target_invariant.abi_encode_input(&[])?.into(),
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

/// Replays and shrinks a call sequence, collecting logs and traces.
///
/// For check mode (target_value=None): shrinks to find shortest failing sequence.
/// For optimization mode (target_value=Some): shrinks to find shortest sequence producing target.
#[expect(clippy::too_many_arguments)]
pub fn replay_error<FEN: FoundryEvmNetwork>(
    config: InvariantConfig,
    mut executor: Executor<FEN>,
    calls: &[BasicTxDetails],
    inner_sequence: Option<Vec<Option<BasicTxDetails>>>,
    expect_assertion_failure: bool,
    target_value: Option<I256>,
    invariant_contract: &InvariantContract<'_>,
    target_invariant: &Function,
    known_contracts: &ContractsByArtifact,
    ided_contracts: ContractsByAddress,
    logs: &mut Vec<Log>,
    traces: &mut Traces,
    line_coverage: &mut Option<HitMaps>,
    deprecated_cheatcodes: &mut HashMap<&'static str, Option<&'static str>>,
    progress: Option<&ProgressBar>,
    early_exit: &EarlyExit,
    position: Option<(usize, usize)>,
) -> Result<Vec<BaseCounterExample>> {
    // Reset progress bar for this invariant's shrink phase. Multi-invariant runs call this once
    // per target so the bar's message reflects which invariant is currently being shrunk and
    // (when more than one invariant needs shrinking) the `[i/N]` counter shows queue depth.
    reset_shrink_progress(&config, progress, &target_invariant.name, position);

    let calls = if let Some(target) = target_value {
        shrink_sequence_value(
            &config,
            invariant_contract,
            target_invariant,
            calls,
            &executor,
            target,
            progress,
            early_exit,
        )?
    } else {
        shrink_sequence(
            &config,
            invariant_contract,
            target_invariant,
            calls,
            expect_assertion_failure,
            &executor,
            progress,
            early_exit,
        )?
    };

    if let Some(sequence) = inner_sequence {
        set_up_inner_replay(&mut executor, &sequence);
    }

    replay_run(
        invariant_contract,
        target_invariant,
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
fn set_up_inner_replay<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    inner_sequence: &[Option<BasicTxDetails>],
) {
    if let Some(fuzzer) = &mut executor.inspector_mut().fuzzer
        && let Some(call_generator) = &mut fuzzer.call_generator
    {
        call_generator.last_sequence = Arc::new(RwLock::new(inner_sequence.to_owned()));
        call_generator.set_replay(true);
    }
}
