use super::{BasicTxDetails, InvariantContract};
use crate::{
    decode::decode_revert,
    executor::{Executor, RawCallResult},
    fuzz::{invariant::set_up_inner_replay, *},
    trace::{load_contracts, TraceKind},
    CALLER,
};
use ethers::{abi::Function, types::Address};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use proptest::test_runner::TestError;

#[derive(Debug, Clone)]
pub struct InvariantFuzzError {
    /// The proptest error occurred as a result of a test case.
    pub test_error: TestError<Vec<BasicTxDetails>>,
    /// The return reason of the offending call.
    pub return_reason: Reason,
    /// The revert string of the offending call.
    pub revert_reason: String,
    /// Address of the invariant asserter.
    pub addr: Address,
    /// Function data for invariant check.
    pub func: Option<ethers::prelude::Bytes>,
    /// Inner fuzzing Sequence coming from overriding calls.
    pub inner_sequence: Vec<Option<BasicTxDetails>>,
}

impl InvariantFuzzError {
    pub fn new(
        invariant_contract: &InvariantContract,
        error_func: Option<&Function>,
        calldata: &[BasicTxDetails],
        call_result: RawCallResult,
        inner_sequence: &[Option<BasicTxDetails>],
    ) -> Self {
        let mut func = None;
        let origin: String;

        if let Some(f) = error_func {
            func = Some(f.short_signature().into());
            origin = f.name.clone();
        } else {
            origin = "Revert".to_string();
        }

        InvariantFuzzError {
            test_error: proptest::test_runner::TestError::Fail(
                format!(
                    "{}, reason: '{}'",
                    origin,
                    match decode_revert(
                        call_result.result.as_ref(),
                        Some(invariant_contract.abi),
                        Some(call_result.status)
                    ) {
                        Ok(e) => e,
                        Err(e) => e.to_string(),
                    }
                )
                .into(),
                calldata.to_vec(),
            ),
            return_reason: "".into(),
            revert_reason: decode_revert(
                call_result.result.as_ref(),
                Some(invariant_contract.abi),
                Some(call_result.status),
            )
            .unwrap_or_default(),
            addr: invariant_contract.address,
            func,
            inner_sequence: inner_sequence.to_vec(),
        }
    }

    /// Replays the error case and collects all necessary traces.
    pub fn replay(
        &self,
        mut executor: Executor,
        known_contracts: Option<&ContractsByArtifact>,
        mut ided_contracts: ContractsByAddress,
        logs: &mut Vec<Log>,
        traces: &mut Vec<(TraceKind, CallTraceArena)>,
    ) -> Option<CounterExample> {
        let mut counterexample_sequence = vec![];
        let calls = match self.test_error {
            // Don't use at the moment.
            TestError::Abort(_) => return None,
            TestError::Fail(_, ref calls) => calls,
        };

        let calls = self.try_shrinking(calls, &executor);

        // We want traces for a failed case.
        executor.set_tracing(true);

        set_up_inner_replay(&mut executor, &self.inner_sequence);

        // Replay each call from the sequence until we break the invariant.
        for (sender, (addr, bytes)) in calls.iter() {
            let call_result = executor
                .call_raw_committing(*sender, *addr, bytes.0.clone(), 0.into())
                .expect("bad call to evm");

            logs.extend(call_result.logs);
            traces.push((TraceKind::Execution, call_result.traces.clone().unwrap()));

            // Identify newly generated contracts, if they exist.
            ided_contracts.extend(load_contracts(
                vec![(TraceKind::Execution, call_result.traces.clone().unwrap())],
                known_contracts,
            ));

            counterexample_sequence.push(BaseCounterExample::create(
                *sender,
                *addr,
                bytes,
                &ided_contracts,
                call_result.traces,
            ));

            // Checks the invariant.
            if let Some(func) = &self.func {
                let error_call_result = executor
                    .call_raw(CALLER, self.addr, func.0.clone(), 0.into())
                    .expect("bad call to evm");

                if error_call_result.reverted {
                    logs.extend(error_call_result.logs);
                    traces.push((TraceKind::Execution, error_call_result.traces.unwrap()));
                    break
                }
            }
        }

        (!counterexample_sequence.is_empty())
            .then_some(CounterExample::Sequence(counterexample_sequence))
    }

    /// Tests that the modified sequence of calls successfully reverts on the error function.
    fn fails_successfully<'a>(
        &self,
        mut executor: Executor,
        calls: &'a [BasicTxDetails],
        anchor: usize,
        removed_calls: &[usize],
    ) -> Result<Vec<&'a BasicTxDetails>, ()> {
        let calls = calls.iter().enumerate().filter_map(|(index, element)| {
            if anchor > index || removed_calls.contains(&index) {
                return None
            }
            Some(element)
        });

        let mut new_sequence = vec![];
        for details in calls {
            new_sequence.push(details);

            let (sender, (addr, bytes)) = details;

            executor
                .call_raw_committing(*sender, *addr, bytes.0.clone(), 0.into())
                .expect("bad call to evm");

            // Checks the invariant. If we exit before the last call, all the better.
            if let Some(func) = &self.func {
                let error_call_result = executor
                    .call_raw(CALLER, self.addr, func.0.clone(), 0.into())
                    .expect("bad call to evm");

                if error_call_result.reverted {
                    return Ok(new_sequence)
                }
            }
        }

        Err(())
    }

    /// Tries to shrink the failure case to its smallest sequence of calls.
    ///
    /// Sets an anchor at the beginning (index=0) and tries to remove all other calls one by one,
    /// until it reaches the last one. The elements which were removed and lead to a failure are
    /// kept in the removal list. The removed ones that didn't lead to a failure are inserted
    /// back into the sequence.
    ///
    /// Once it reaches the end, it increments the anchor, resets the removal list and starts the
    /// same process again.
    ///
    /// Returns the smallest sequence found.
    fn try_shrinking<'a>(
        &self,
        calls: &'a [BasicTxDetails],
        executor: &Executor,
    ) -> Vec<&'a BasicTxDetails> {
        let mut anchor = 0;
        let mut removed_calls = vec![];
        let mut shrinked = calls.iter().collect::<Vec<_>>();

        while anchor != calls.len() {
            // Get the latest removed element, so we know which one to remove next.
            let removed =
                match self.fails_successfully(executor.clone(), calls, anchor, &removed_calls) {
                    Ok(new_sequence) => {
                        if shrinked.len() > new_sequence.len() {
                            shrinked = new_sequence;
                        }
                        removed_calls.last().cloned()
                    }
                    Err(_) => removed_calls.pop(),
                };

            if let Some(last_removed) = removed {
                // If we haven't reached the end of the sequence, then remove the next element.
                // Otherwise, restart the process with an incremented anchor.

                let next_removed = last_removed + 1;

                if next_removed > calls.len() - 1 {
                    anchor += 1;
                    removed_calls = vec![];
                    continue
                }

                removed_calls.push(next_removed);
            } else {
                // When the process is restarted, `removed_calls` will be empty.
                removed_calls.push(anchor + 1);
            }
        }

        shrinked
    }
}
