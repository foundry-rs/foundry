use super::{error::FailedInvariantCaseData, InvariantFailures, InvariantFuzzError};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::Log;
use eyre::Result;
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_config::InvariantConfig;
use foundry_evm_core::constants::CALLER;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::{
    invariant::{BasicTxDetails, FuzzRunIdentifiedContracts, InvariantContract},
    BaseCounterExample, CounterExample,
};
use foundry_evm_traces::{load_contracts, TraceKind, Traces};
use revm::primitives::U256;
use std::borrow::Cow;

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
/// Either returns the call result if successful, or nothing if there was an error.
pub fn assert_invariants(
    invariant_contract: &InvariantContract<'_>,
    invariant_config: &InvariantConfig,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
) -> Result<Option<RawCallResult>> {
    let mut inner_sequence = vec![];

    if let Some(fuzzer) = &executor.inspector.fuzzer {
        if let Some(call_generator) = &fuzzer.call_generator {
            inner_sequence.extend(call_generator.last_sequence.read().iter().cloned());
        }
    }

    let func = invariant_contract.invariant_function;
    let mut call_result = executor.call_raw(
        CALLER,
        invariant_contract.address,
        func.abi_encode_input(&[]).expect("invariant should have no inputs").into(),
        U256::ZERO,
    )?;

    let is_err = !executor.is_raw_call_success(
        invariant_contract.address,
        Cow::Owned(call_result.state_changeset.take().unwrap()),
        &call_result,
        false,
    );
    if is_err {
        // We only care about invariants which we haven't broken yet.
        if invariant_failures.error.is_none() {
            let case_data = FailedInvariantCaseData::new(
                invariant_contract,
                invariant_config,
                targeted_contracts,
                calldata,
                call_result,
                &inner_sequence,
            );
            invariant_failures.error = Some(InvariantFuzzError::BrokenInvariant(case_data));
            return Ok(None);
        }
    }

    Ok(Some(call_result))
}

/// Replays a call sequence for collecting logs and traces.
/// Returns counterexample to be used when the call sequence is a failed scenario.
#[allow(clippy::too_many_arguments)]
pub fn replay_run(
    invariant_contract: &InvariantContract<'_>,
    mut executor: Executor,
    known_contracts: &ContractsByArtifact,
    mut ided_contracts: ContractsByAddress,
    logs: &mut Vec<Log>,
    traces: &mut Traces,
    coverage: &mut Option<HitMaps>,
    inputs: Vec<BasicTxDetails>,
) -> Result<Option<CounterExample>> {
    // We want traces for a failed case.
    executor.set_tracing(true);

    let mut counterexample_sequence = vec![];

    // Replay each call from the sequence, collect logs, traces and coverage.
    for (sender, (addr, bytes)) in inputs.iter() {
        let call_result =
            executor.call_raw_committing(*sender, *addr, bytes.clone(), U256::ZERO)?;
        logs.extend(call_result.logs);
        traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));

        if let Some(new_coverage) = call_result.coverage {
            if let Some(old_coverage) = coverage {
                *coverage = Some(std::mem::take(old_coverage).merge(new_coverage));
            } else {
                *coverage = Some(new_coverage);
            }
        }

        // Identify newly generated contracts, if they exist.
        ided_contracts.extend(load_contracts(
            vec![(TraceKind::Execution, call_result.traces.clone().unwrap())],
            known_contracts,
        ));

        // Create counter example to be used in failed case.
        counterexample_sequence.push(BaseCounterExample::create(
            *sender,
            *addr,
            bytes,
            &ided_contracts,
            call_result.traces,
        ));

        // Replay invariant to collect logs and traces.
        let error_call_result = executor.call_raw(
            CALLER,
            invariant_contract.address,
            invariant_contract
                .invariant_function
                .abi_encode_input(&[])
                .expect("invariant should have no inputs")
                .into(),
            U256::ZERO,
        )?;
        traces.push((TraceKind::Execution, error_call_result.traces.clone().unwrap()));
        logs.extend(error_call_result.logs);
    }

    Ok((!counterexample_sequence.is_empty())
        .then_some(CounterExample::Sequence(counterexample_sequence)))
}
