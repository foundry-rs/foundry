use crate::executors::{Executor, RawCallResult};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::{Function, JsonAbi as Abi};
use alloy_primitives::{Address, Bytes, U256};
use eyre::Result;
use foundry_config::FuzzConfig;
use foundry_evm_coverage::HitMaps;
use foundry_evm_executors::{
    decode::{self, decode_console_logs},
    ASSUME_MAGIC_RETURN_CODE,
};
use foundry_evm_fuzz::{
    strategies::{
        build_initial_state, collect_state_from_call, fuzz_calldata, fuzz_calldata_from_state,
        EvmFuzzState,
    },
    BaseCounterExample, CounterExample, FuzzCase, FuzzError, FuzzTestResult,
};
use foundry_evm_traces::CallTraceArena;
use proptest::test_runner::{TestCaseError, TestError, TestRunner};
use std::cell::RefCell;

mod types;
pub use types::{CaseOutcome, CounterExampleOutcome, FuzzOutcome};

/// Wrapper around an [`Executor`] which provides fuzzing support using [`proptest`].
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](proptest::test_runner::Config)
pub struct FuzzedExecutor<'a> {
    /// The VM
    executor: &'a Executor,
    /// The fuzzer
    runner: TestRunner,
    /// The account that calls tests
    sender: Address,
    /// The fuzz configuration
    config: FuzzConfig,
}

impl<'a> FuzzedExecutor<'a> {
    /// Instantiates a fuzzed executor given a testrunner
    pub fn new(
        executor: &'a Executor,
        runner: TestRunner,
        sender: Address,
        config: FuzzConfig,
    ) -> Self {
        Self { executor, runner, sender, config }
    }

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case
    pub fn fuzz(
        &self,
        func: &Function,
        address: Address,
        should_fail: bool,
        errors: Option<&Abi>,
    ) -> FuzzTestResult {
        // Stores the first Fuzzcase
        let first_case: RefCell<Option<FuzzCase>> = RefCell::default();

        // gas usage per case
        let gas_by_case: RefCell<Vec<(u64, u64)>> = RefCell::default();

        // Stores the result and calldata of the last failed call, if any.
        let counterexample: RefCell<(Bytes, RawCallResult)> = RefCell::default();

        // Stores the last successful call trace
        let traces: RefCell<Option<CallTraceArena>> = RefCell::default();

        // Stores coverage information for all fuzz cases
        let coverage: RefCell<Option<HitMaps>> = RefCell::default();

        let state = self.build_fuzz_state();

        let mut weights = vec![];
        let dictionary_weight = self.config.dictionary.dictionary_weight.min(100);
        if self.config.dictionary.dictionary_weight < 100 {
            weights.push((100 - dictionary_weight, fuzz_calldata(func.clone())));
        }
        if dictionary_weight > 0 {
            weights.push((
                self.config.dictionary.dictionary_weight,
                fuzz_calldata_from_state(func.clone(), state.clone()),
            ));
        }

        let strat = proptest::strategy::Union::new_weighted(weights);
        debug!(func = ?func.name, should_fail, "fuzzing");
        let run_result = self.runner.clone().run(&strat, |calldata| {
            let fuzz_res = self.single_fuzz(&state, address, should_fail, calldata)?;

            match fuzz_res {
                FuzzOutcome::Case(case) => {
                    let mut first_case = first_case.borrow_mut();
                    gas_by_case.borrow_mut().push((case.case.gas, case.case.stipend));
                    if first_case.is_none() {
                        first_case.replace(case.case);
                    }

                    traces.replace(case.traces);

                    if let Some(prev) = coverage.take() {
                        // Safety: If `Option::or` evaluates to `Some`, then `call.coverage` must
                        // necessarily also be `Some`
                        coverage.replace(Some(prev.merge(case.coverage.unwrap())));
                    } else {
                        coverage.replace(case.coverage);
                    }

                    Ok(())
                }
                FuzzOutcome::CounterExample(CounterExampleOutcome {
                    exit_reason,
                    counterexample: _counterexample,
                    ..
                }) => {
                    let status = exit_reason;
                    // We cannot use the calldata returned by the test runner in `TestError::Fail`,
                    // since that input represents the last run case, which may not correspond with
                    // our failure - when a fuzz case fails, proptest will try
                    // to run at least one more case to find a minimal failure
                    // case.
                    let call_res = _counterexample.1.result.clone();
                    *counterexample.borrow_mut() = _counterexample;
                    Err(TestCaseError::fail(
                        decode::decode_revert(&call_res, errors, Some(status)).unwrap_or_default(),
                    ))
                }
            }
        });

        let (calldata, call) = counterexample.into_inner();
        let mut result = FuzzTestResult {
            first_case: first_case.take().unwrap_or_default(),
            gas_by_case: gas_by_case.take(),
            success: run_result.is_ok(),
            reason: None,
            counterexample: None,
            decoded_logs: decode_console_logs(&call.logs),
            logs: call.logs,
            labeled_addresses: call.labels,
            traces: if run_result.is_ok() { traces.into_inner() } else { call.traces.clone() },
            coverage: coverage.into_inner(),
        };

        match run_result {
            // Currently the only operation that can trigger proptest global rejects is the
            // `vm.assume` cheatcode, thus we surface this info to the user when the fuzz test
            // aborts due to too many global rejects, making the error message more actionable.
            Err(TestError::Abort(reason)) if reason.message() == "Too many global rejects" => {
                result.reason = Some(
                    FuzzError::TooManyRejects(self.runner.config().max_global_rejects).to_string(),
                );
            }
            Err(TestError::Abort(reason)) => {
                result.reason = Some(reason.to_string());
            }
            Err(TestError::Fail(reason, _)) => {
                let reason = reason.to_string();
                result.reason = if reason.is_empty() { None } else { Some(reason) };

                let args =
                    func.abi_decode_input(&calldata.as_ref()[4..], false).unwrap_or_default();
                result.counterexample = Some(CounterExample::Single(BaseCounterExample {
                    sender: None,
                    addr: None,
                    signature: None,
                    contract_name: None,
                    traces: call.traces,
                    calldata,
                    args,
                }));
            }
            _ => {}
        }

        result
    }

    /// Granular and single-step function that runs only one fuzz and returns either a `CaseOutcome`
    /// or a `CounterExampleOutcome`
    pub fn single_fuzz(
        &self,
        state: &EvmFuzzState,
        address: Address,
        should_fail: bool,
        calldata: alloy_primitives::Bytes,
    ) -> Result<FuzzOutcome, TestCaseError> {
        let call = self
            .executor
            .call_raw(self.sender, address, calldata.clone(), U256::ZERO)
            .map_err(|_| TestCaseError::fail(FuzzError::FailedContractCall))?;
        let state_changeset = call
            .state_changeset
            .as_ref()
            .ok_or_else(|| TestCaseError::fail(FuzzError::EmptyChangeset))?;

        // Build fuzzer state
        collect_state_from_call(
            &call.logs,
            state_changeset,
            state.clone(),
            &self.config.dictionary,
        );

        // When assume cheat code is triggered return a special string "FOUNDRY::ASSUME"
        if call.result.as_ref() == ASSUME_MAGIC_RETURN_CODE {
            return Err(TestCaseError::reject(FuzzError::AssumeReject))
        }

        let breakpoints = call
            .cheatcodes
            .as_ref()
            .map_or_else(Default::default, |cheats| cheats.breakpoints.clone());

        let success =
            self.executor.is_success(address, call.reverted, state_changeset.clone(), should_fail);

        if success {
            Ok(FuzzOutcome::Case(CaseOutcome {
                case: FuzzCase { calldata, gas: call.gas_used, stipend: call.stipend },
                traces: call.traces,
                coverage: call.coverage,
                debug: call.debug,
                breakpoints,
            }))
        } else {
            Ok(FuzzOutcome::CounterExample(CounterExampleOutcome {
                debug: call.debug.clone(),
                exit_reason: call.exit_reason,
                counterexample: (calldata, call),
                breakpoints,
            }))
        }
    }

    /// Stores fuzz state for use with [fuzz_calldata_from_state]
    pub fn build_fuzz_state(&self) -> EvmFuzzState {
        if let Some(fork_db) = self.executor.backend.active_fork_db() {
            build_initial_state(fork_db, &self.config.dictionary)
        } else {
            build_initial_state(self.executor.backend.mem_db(), &self.config.dictionary)
        }
    }
}
