//! Test config.

use forge::{
    result::{SuiteResult, TestStatus},
    MultiContractRunner,
};
use foundry_evm::{
    decode::decode_console_logs,
    revm::primitives::SpecId,
    traces::{decode_trace_arena, render_trace_arena, CallTraceDecoderBuilder},
};
use foundry_test_utils::{init_tracing, Filter};
use futures::future::join_all;
use itertools::Itertools;
use std::collections::BTreeMap;

/// How to execute a test run.
pub struct TestConfig {
    pub runner: MultiContractRunner,
    pub should_fail: bool,
    pub filter: Filter,
}

impl TestConfig {
    pub fn new(runner: MultiContractRunner) -> Self {
        Self::with_filter(runner, Filter::matches_all())
    }

    pub fn with_filter(runner: MultiContractRunner, filter: Filter) -> Self {
        init_tracing();
        Self { runner, should_fail: false, filter }
    }

    pub fn evm_spec(mut self, spec: SpecId) -> Self {
        self.runner.evm_spec = spec;
        self
    }

    pub fn should_fail(self) -> Self {
        self.set_should_fail(true)
    }

    pub fn set_should_fail(mut self, should_fail: bool) -> Self {
        self.should_fail = should_fail;
        self
    }

    /// Executes the test runner
    pub fn test(&mut self) -> BTreeMap<String, SuiteResult> {
        self.runner.test_collect(&self.filter)
    }

    pub async fn run(&mut self) {
        self.try_run().await.unwrap()
    }

    /// Executes the test case
    ///
    /// Returns an error if
    ///    * filter matched 0 test cases
    ///    * a test results deviates from the configured `should_fail` setting
    pub async fn try_run(&mut self) -> eyre::Result<()> {
        let suite_result = self.test();
        if suite_result.is_empty() {
            eyre::bail!("empty test result");
        }
        for (_, SuiteResult { test_results, .. }) in suite_result {
            for (test_name, mut result) in test_results {
                if self.should_fail && (result.status == TestStatus::Success) ||
                    !self.should_fail && (result.status == TestStatus::Failure)
                {
                    let logs = decode_console_logs(&result.logs);
                    let outcome = if self.should_fail { "fail" } else { "pass" };
                    let call_trace_decoder = CallTraceDecoderBuilder::default()
                        .with_known_contracts(&self.runner.known_contracts)
                        .build();
                    let decoded_traces = join_all(result.traces.iter_mut().map(|(_, arena)| {
                        let decoder = &call_trace_decoder;
                        async move {
                            decode_trace_arena(arena, decoder)
                                .await
                                .expect("Failed to decode traces");
                            render_trace_arena(arena)
                        }
                    }))
                    .await
                    .into_iter()
                    .collect::<Vec<String>>();
                    eyre::bail!(
                        "Test {} did not {} as expected.\nReason: {:?}\nLogs:\n{}\n\nTraces:\n{}",
                        test_name,
                        outcome,
                        result.reason,
                        logs.join("\n"),
                        decoded_traces.into_iter().format("\n"),
                    )
                }
            }
        }

        Ok(())
    }
}

/// A helper to assert the outcome of multiple tests with helpful assert messages
#[track_caller]
#[allow(clippy::type_complexity)]
pub fn assert_multiple(
    actuals: &BTreeMap<String, SuiteResult>,
    expecteds: BTreeMap<
        &str,
        Vec<(&str, bool, Option<String>, Option<Vec<String>>, Option<usize>)>,
    >,
) {
    assert_eq!(actuals.len(), expecteds.len(), "We did not run as many contracts as we expected");
    for (contract_name, tests) in &expecteds {
        assert!(
            actuals.contains_key(*contract_name),
            "We did not run the contract {contract_name}"
        );

        assert_eq!(
            actuals[*contract_name].len(),
            expecteds[contract_name].len(),
            "We did not run as many test functions as we expected for {contract_name}"
        );
        for (test_name, should_pass, reason, expected_logs, expected_warning_count) in tests {
            let logs = &decode_console_logs(&actuals[*contract_name].test_results[*test_name].logs);

            let warnings_count = &actuals[*contract_name].warnings.len();

            if *should_pass {
                assert!(
                    actuals[*contract_name].test_results[*test_name].status == TestStatus::Success,
                    "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    actuals[*contract_name].test_results[*test_name].reason,
                    logs.join("\n")
                );
            } else {
                assert!(
                    actuals[*contract_name].test_results[*test_name].status == TestStatus::Failure,
                    "Test {} did not fail as expected.\nLogs:\n{}",
                    test_name,
                    logs.join("\n")
                );
                assert_eq!(
                    actuals[*contract_name].test_results[*test_name].reason, *reason,
                    "Failure reason for test {test_name} did not match what we expected."
                );
            }

            if let Some(expected_logs) = expected_logs {
                assert_eq!(
                    logs,
                    expected_logs,
                    "Logs did not match for test {}.\nExpected:\n{}\n\nGot:\n{}",
                    test_name,
                    expected_logs.join("\n"),
                    logs.join("\n")
                );
            }

            if let Some(expected_warning_count) = expected_warning_count {
                assert_eq!(
                    warnings_count, expected_warning_count,
                    "Test {test_name} did not pass as expected. Expected:\n{expected_warning_count}Got:\n{warnings_count}"
                );
            }
        }
    }
}
