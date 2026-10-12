//! Native fuzz cases over independently owned setup state.

use super::*;
use crate::test_config::{fuzz_test_path_name, fuzzer_with_cases};
use alloy_dyn_abi::{DynSolValue, FunctionExt, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::map::HashMap;
use foundry_evm::constants::MAGIC_ASSUME;
use foundry_evm_fuzz::{
    BaseCounterExample, BasicTxDetails, CounterExample, FuzzCase, FuzzFixtures, FuzzRunMetadata,
    FuzzTestResult, fixture_name,
    strategies::{EvmFuzzState, TxGenerator},
};
use proptest::test_runner::{TestCaseError, TestError};
use std::cell::{Cell, RefCell};

impl EthereumRunner {
    pub(super) fn fuzz_fixtures(
        &self,
        executor: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
        abi: &JsonAbi,
    ) -> Result<FuzzFixtures> {
        let mut fixtures = HashMap::default();
        for function in abi.functions().filter(|function| function.is_fixture()) {
            let mut values = Vec::new();
            let mut index = 0u64;
            loop {
                let arguments = if function.inputs.is_empty() {
                    Vec::new()
                } else {
                    vec![DynSolValue::Uint(U256::from(index), 256)]
                };
                let Ok(input) = function.abi_encode_input(&arguments) else { break };
                let case = executor.clone();
                let (result, _) = case.inspect_call(
                    &self.tx(&case, CALLER, TxKind::Call(address), input.into())?,
                    cheats.clone(),
                );
                let result = result?;
                if !result.status {
                    tracing::debug!(fixture = function.name, output = ?result.output, "native fuzz fixture reverted");
                    break;
                }
                let Ok(mut outputs) = function.abi_decode_output(&result.output) else { break };
                if outputs.len() != 1 {
                    break;
                }
                let value = outputs.pop().expect("one fixture output");
                if function.inputs.is_empty() {
                    fixtures.insert(fixture_name(function.name.clone()), value);
                    break;
                }
                values.push(value);
                index += 1;
            }
            if !function.inputs.is_empty() {
                fixtures.insert(fixture_name(function.name.clone()), DynSolValue::Array(values));
            }
        }
        Ok(FuzzFixtures::new(fixtures).with_enum_bounds(self.artifacts.enum_bounds.clone()))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Native case execution uses the prepared contract and setup state."
    )]
    pub(super) fn fuzz(
        &self,
        executor: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
        function: &Function,
        fixtures: &FuzzFixtures,
        env: &EthereumEnv,
        config: &Config,
        contract: &str,
        abi: &JsonAbi,
    ) -> Result<TestResult> {
        let legacy_assertions = config.legacy_assertions && abi.functions.contains_key("failed");
        let config = &config.fuzz;
        ensure!(
            config.corpus.corpus_dir.is_none(),
            "native EVM2 fuzz corpus replay is not migrated yet"
        );
        ensure!(
            config.run.is_none() && config.worker.is_none() && config.timeout.is_none(),
            "native EVM2 fuzz run selection/timeouts are not migrated yet"
        );
        let timer = Instant::now();
        let mut runner = fuzzer_with_cases(config.seed, config.runs, config.max_test_rejects);
        let mut seed = EvmFuzzState::empty(
            &self.artifacts.library_addresses,
            config.dictionary,
            Some(&self.artifacts.fuzz_literals),
        );
        let cache = executor.accepted_cache();
        let mut accounts = cache.accounts.iter().collect::<Vec<_>>();
        accounts.sort_unstable_by_key(|(address, _)| **address);
        for (address, account) in accounts {
            if let Some(account) = account {
                let code = config
                    .dictionary
                    .include_push_bytes
                    .then(|| executor.code(*address))
                    .transpose()?;
                let storage =
                    cache.storage.get(address).into_iter().flat_map(|storage| {
                        storage.slots.iter().map(|(slot, value)| (*slot, *value))
                    });
                seed.seed_account(
                    *address,
                    code.as_ref().map(|code| (account.code_hash, code.original_byte_slice())),
                    storage,
                );
            }
        }
        let strategy = TxGenerator::stateless(
            seed,
            fixtures.clone(),
            address,
            self.opts.sender,
            function.clone(),
            config.dictionary.dictionary_weight,
            config.corpus.payable_value_weight,
        )
        .into_strategy();
        let result = RefCell::new(FuzzTestResult { success: true, ..Default::default() });
        let fatal = RefCell::<Option<eyre::Report>>::new(None);
        let attempts = Cell::new(0u32);
        let execute = |input: Bytes, value: Option<U256>| {
            attempts.set(attempts.get() + 1);
            let mut case = executor.clone();
            let mut tx =
                match self.tx(&case, self.opts.sender, TxKind::Call(address), input.clone()) {
                    Ok(tx) => tx,
                    Err(error) => {
                        *fatal.borrow_mut() = Some(error);
                        return Err(TestCaseError::fail("native sender state read failed"));
                    }
                };
            if let TxEnvelope::Legacy(tx) = tx.inner_mut() {
                tx.value = value.unwrap_or_default();
            }
            let (executed, mut observations) = case.inspect_transact(&tx, cheats.clone());
            let executed = match executed {
                Ok(executed) => executed,
                Err(error) => {
                    *fatal.borrow_mut() = Some(error.into());
                    return Err(TestCaseError::fail("native execution failed"));
                }
            };
            HitMaps::merge_opt(
                &mut result.borrow_mut().line_coverage,
                observations.line_coverage.take().map(|coverage| coverage.finish()),
            );
            if executed.output.as_ref() == MAGIC_ASSUME {
                return Err(TestCaseError::reject("vm.assume rejected the case"));
            }
            let gas = executed.tx_gas_used();
            let stipend = self.stipend(env, address, &input);
            let assertion_failed = match case.storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT) {
                Ok(value) => value != U256::ZERO,
                Err(error) => {
                    *fatal.borrow_mut() = Some(error.into());
                    return Err(TestCaseError::fail("native state read failed"));
                }
            };
            let ignore_revert = foundry_evm_fuzz::should_ignore_revert(
                config.fail_on_revert,
                address,
                observations.reverter,
                &[],
            );
            let legacy_failed = if !ignore_revert
                && executed.status
                && !assertion_failed
                && !observations.snapshot_failure
                && legacy_assertions
            {
                match self.legacy_failed(&case, &observations, address) {
                    Ok(failed) => failed,
                    Err(error) => {
                        *fatal.borrow_mut() = Some(error);
                        return Err(TestCaseError::fail("native legacy state read failed"));
                    }
                }
            } else {
                false
            };
            let mut test =
                self.outcome(executed, assertion_failed, stipend, observations, timer.elapsed());
            if ignore_revert && !test.status.is_skipped() {
                test.status = TestStatus::Success;
                test.reason = None;
            } else if test.status.is_success() && legacy_failed {
                test.status = TestStatus::Failure;
            }
            let mut result = result.borrow_mut();
            if result.gas_by_case.is_empty() {
                result.first_case = FuzzCase { gas, stipend };
            }
            if test.status.is_success() {
                result.gas_by_case.push((gas, stipend));
                if config.show_logs {
                    result.logs.extend(test.logs);
                }
                return Ok(());
            }
            result.success = false;
            result.skipped = test.status.is_skipped();
            result.reason = test.reason;
            result.logs = test.logs;
            Err(TestCaseError::fail("native fuzz case did not pass"))
        };
        let test_name = fuzz_test_path_name(abi, function, config, contract);
        let failure_dir = config.failure_persist_dir.as_ref().map(|dir| {
            dir.join("failures").join(contract.split(':').next_back().unwrap_or(contract))
        });
        let failure_file = failure_dir.as_ref().map(|dir| dir.join(test_name.as_ref()));
        let persisted = failure_file
            .as_ref()
            .and_then(|path| {
                foundry_common::fs::read_json_file::<BaseCounterExample>(path).ok().or_else(|| {
                    (test_name != function.name)
                        .then(|| {
                            foundry_common::fs::read_json_file::<BaseCounterExample>(
                                &failure_dir
                                    .as_ref()
                                    .expect("failure directory")
                                    .join(&function.name),
                            )
                            .ok()
                        })
                        .flatten()
                })
            })
            .filter(|failure| {
                failure.calldata.get(..4).is_some_and(|selector| function.selector() == selector)
                    && function.abi_decode_input(&failure.calldata[4..]).is_ok()
            });
        let replayed_failure = persisted.filter(|failure| {
            let outcome = execute(failure.calldata.clone(), failure.value);
            // A successful or rejected old counterexample must not consume a fresh run.
            if outcome.is_ok() {
                let mut result = result.borrow_mut();
                result.gas_by_case.clear();
                result.logs.clear();
            }
            attempts.set(0);
            matches!(outcome, Err(TestCaseError::Fail(_)))
        });
        let outcome = if fatal.borrow().is_some() {
            Err(TestError::Abort("native execution failed".into()))
        } else if let Some(failure) = &replayed_failure {
            Err(TestError::Fail(
                "persisted fuzz failure".into(),
                BasicTxDetails {
                    warp: None,
                    roll: None,
                    sender: self.opts.sender,
                    call_details: foundry_evm_fuzz::CallDetails {
                        target: address,
                        calldata: failure.calldata.clone(),
                        value: failure.value,
                    },
                },
            ))
        } else {
            runner.run(&strategy, |input| {
                execute(input.call_details.calldata, input.call_details.value)
            })
        };
        if let Some(error) = fatal.into_inner() {
            return Err(error);
        }
        let mut result = result.into_inner();
        match outcome {
            Ok(()) => {}
            Err(TestError::Fail(_, input)) if !result.skipped => {
                let value = input.call_details.value;
                let input = input.call_details.calldata;
                let arguments = function.abi_decode_input(&input[4..])?;
                let mut counterexample = BaseCounterExample::from_fuzz_call(input, arguments, None);
                if let Some(failure) = replayed_failure {
                    counterexample.value = failure.value;
                    counterexample.fuzz = failure.fuzz;
                } else {
                    counterexample.value = value;
                    counterexample.fuzz =
                        FuzzRunMetadata::new(config.seed, Some(attempts.get()), Some(0));
                }
                result.counterexample = Some(CounterExample::Single(counterexample));
            }
            Err(TestError::Abort(reason)) => {
                result.success = false;
                result.reason = Some(reason.to_string());
            }
            Err(TestError::Fail(..)) => {}
        }
        if let Some(CounterExample::Single(counterexample)) = &result.counterexample
            && let Some(path) = &failure_file
            && let Err(error) =
                foundry_common::fs::create_dir_all(failure_dir.as_ref().expect("failure directory"))
                    .and_then(|()| foundry_common::fs::write_json_file(path, counterexample))
        {
            tracing::error!(%error, "Failed to record fuzz counterexample");
        }
        let mut test = TestResult::default();
        test.fuzz_result(result);
        test.duration = timer.elapsed();
        Ok(test)
    }
}
