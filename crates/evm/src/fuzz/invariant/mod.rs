//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used

use crate::{
    executor::{Executor, OnLog},
    fuzz::*,
    trace::{load_contracts, TraceKind, Traces},
    CALLER,
};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes, U256},
};
use foundry_common::ContractsByArtifact;
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};

pub use proptest::test_runner::Config as FuzzConfig;

mod error;
pub use error::InvariantFuzzError;

mod call_override;
pub use call_override::{set_up_inner_replay, RandomCallGenerator};

mod executor;
pub use executor::{InvariantExecutor, InvariantFailures};

mod filters;
pub use filters::{ArtifactFilters, SenderFilters};

pub type TargetedContracts = BTreeMap<Address, (String, Abi, Vec<Function>)>;
pub type FuzzRunIdentifiedContracts = Arc<Mutex<TargetedContracts>>;

/// (Sender, (TargetContract, Calldata))
pub type BasicTxDetails = (Address, (Address, Bytes));

/// Test contract which is testing its invariants.
#[derive(Debug, Clone)]
pub struct InvariantContract<'a> {
    /// Address of the test contract.
    pub address: Address,
    /// Invariant functions present in the test contract.
    pub invariant_function: &'a Function,
    /// Abi of the test contract.
    pub abi: &'a Abi,
}

/// Given the executor state, asserts that no invariant has been broken. Otherwise, it fills the
/// external `invariant_failures.failed_invariant` map and returns a generic error.
/// Returns the mapping of (Invariant Function Name -> Call Result).
pub fn assert_invariants<ONLOG: OnLog>(
    invariant_contract: &InvariantContract,
    executor: &Executor<ONLOG>,
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
            func.encode_input(&[]).expect("invariant should have no inputs").into(),
            U256::zero(),
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
pub fn replay_run<ONLOG: OnLog>(
    invariant_contract: &InvariantContract,
    mut executor: Executor<ONLOG>,
    known_contracts: Option<&ContractsByArtifact>,
    mut ided_contracts: ContractsByAddress,
    logs: &mut Vec<Log>,
    traces: &mut Traces,
    func: Function,
    inputs: Vec<BasicTxDetails>,
) {
    // We want traces for a failed case.
    executor.set_tracing(true);

    // set_up_inner_replay(&mut executor, &inputs);

    // Replay each call from the sequence until we break the invariant.
    for (sender, (addr, bytes)) in inputs.iter() {
        let call_result = executor
            .call_raw_committing(*sender, *addr, bytes.0.clone(), U256::zero())
            .expect("bad call to evm");

        logs.extend(call_result.logs);
        traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));

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
                func.encode_input(&[]).expect("invariant should have no inputs").into(),
                U256::zero(),
            )
            .expect("bad call to evm");

        traces.push((TraceKind::Execution, error_call_result.traces.clone().unwrap()));

        logs.extend(error_call_result.logs);
    }
}

/// The outcome of an invariant fuzz test
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    pub error: Option<InvariantFuzzError>,
    /// Every successful fuzz test case
    pub cases: Vec<FuzzedCases>,
    /// Number of reverted fuzz calls
    pub reverts: usize,

    /// The entire inputs of the last run of the invariant campaign, used for
    /// replaying the run for collecting traces.
    pub last_run_inputs: Vec<BasicTxDetails>,
}
