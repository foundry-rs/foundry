use super::{InvariantFailures, InvariantFuzzError};
use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use ethers_core::types::Log;
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::constants::CALLER;
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::invariant::{BasicTxDetails, InvariantContract};
use foundry_evm_traces::{load_contracts, TraceKind, Traces};
use revm::primitives::U256;

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
/// Either returns the call result if successful, or nothing if there was an error.
pub fn assert_invariants(
    invariant_contract: &InvariantContract<'_>,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    invariant_failures: &mut InvariantFailures,
    shrink_sequence: bool,
) -> Option<RawCallResult> {
    let mut inner_sequence = vec![];

    if let Some(fuzzer) = &executor.inspector.fuzzer {
        if let Some(call_generator) = &fuzzer.call_generator {
            inner_sequence.extend(call_generator.last_sequence.read().iter().cloned());
        }
    }

    let func = invariant_contract.invariant_function;
    let mut call_result = executor
        .call_raw(
            CALLER,
            invariant_contract.address,
            func.abi_encode_input(&[]).expect("invariant should have no inputs").into(),
            U256::ZERO,
        )
        .expect("EVM error");

    // This will panic and get caught by the executor
    let is_err = call_result.reverted ||
        !executor.is_success(
            invariant_contract.address,
            call_result.reverted,
            call_result.state_changeset.take().expect("we should have a state changeset"),
            false,
        );
    if is_err {
        // We only care about invariants which we haven't broken yet.
        if invariant_failures.error.is_none() {
            invariant_failures.error = Some(InvariantFuzzError::new(
                invariant_contract,
                Some(func),
                calldata,
                call_result,
                &inner_sequence,
                shrink_sequence,
            ));
            return None
        }
    }

    Some(call_result)
}

/// Replays the provided invariant run for collecting the logs and traces from all depths.
#[allow(clippy::too_many_arguments)]
pub fn replay_run(
    invariant_contract: &InvariantContract<'_>,
    mut executor: Executor,
    known_contracts: Option<&ContractsByArtifact>,
    mut ided_contracts: ContractsByAddress,
    logs: &mut Vec<Log>,
    traces: &mut Traces,
    coverage: &mut Option<HitMaps>,
    func: Function,
    inputs: Vec<BasicTxDetails>,
) {
    // We want traces for a failed case.
    executor.set_tracing(true);

    // set_up_inner_replay(&mut executor, &inputs);

    // Replay each call from the sequence until we break the invariant.
    for (sender, (addr, bytes)) in inputs.iter() {
        let call_result = executor
            .call_raw_committing(*sender, *addr, bytes.clone(), U256::ZERO)
            .expect("bad call to evm");

        logs.extend(call_result.logs);
        traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));

        let old_coverage = std::mem::take(coverage);
        match (old_coverage, call_result.coverage) {
            (Some(old_coverage), Some(call_coverage)) => {
                *coverage = Some(old_coverage.merge(call_coverage));
            }
            (None, Some(call_coverage)) => {
                *coverage = Some(call_coverage);
            }
            (Some(old_coverage), None) => {
                *coverage = Some(old_coverage);
            }
            (None, None) => {}
        }

        // Identify newly generated contracts, if they exist.
        ided_contracts.extend(load_contracts(
            vec![(TraceKind::Execution, call_result.traces.clone().unwrap())],
            known_contracts,
        ));

        // Checks the invariant.
        let error_call_result = executor
            .call_raw(
                CALLER,
                invariant_contract.address,
                func.abi_encode_input(&[]).expect("invariant should have no inputs").into(),
                U256::ZERO,
            )
            .expect("bad call to evm");

        traces.push((TraceKind::Execution, error_call_result.traces.clone().unwrap()));

        logs.extend(error_call_result.logs);
    }
}
