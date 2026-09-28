//! Stateless fuzz cases executed against the native post-setup EVM state.

use super::{EthereumContractRunner, EthereumEnv, HitMaps, TestResult, U256};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::Log;
use evm2::{ethereum::intrinsic_gas, evm::Database};
use eyre::{Result, ensure};
use foundry_config::FuzzConfig;
use foundry_evm::{
    core::{
        constants::{CALLER, MAGIC_ASSUME},
        decode::RevertDecoder,
    },
    fuzz::{
        BaseCounterExample, CounterExample, FuzzCase, FuzzFixtures, FuzzTestResult,
        strategies::fuzz_calldata,
    },
};
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestError, TestRng, TestRunner};
use std::{cell::RefCell, time::Instant};

#[derive(Default)]
struct Cases {
    gas: Vec<(u64, u64)>,
    first: Option<FuzzCase>,
    logs: Vec<Log>,
    skipped: Option<String>,
    coverage: Option<HitMaps>,
}

pub(super) fn run<D: Database + Clone + 'static>(
    runner: &EthereumContractRunner<D>,
    function: &Function,
    config: &FuzzConfig,
    decoder: &RevertDecoder,
    env: &EthereumEnv,
) -> Result<TestResult> {
    ensure!(config.run.is_none(), "single-run fuzz replay is not yet supported with evm2");
    let start = Instant::now();
    let cases = RefCell::new(Cases::default());
    let proptest_config = Config {
        cases: config.runs,
        max_global_rejects: config.max_test_rejects,
        max_shrink_iters: 0,
        failure_persistence: None,
        ..Default::default()
    };
    let mut proptest = if let Some(seed) = config.seed {
        TestRunner::new_with_rng(
            proptest_config,
            TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>()),
        )
    } else {
        TestRunner::new(proptest_config)
    };
    let strategy = fuzz_calldata(function.clone(), &FuzzFixtures::default());
    let outcome = proptest.run(&strategy, |calldata| {
        let mut execution = runner
            .run_test(calldata.clone(), U256::ZERO)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        if !execution.result.status && execution.result.output.as_ref() == MAGIC_ASSUME {
            return Err(TestCaseError::reject("vm.assume"));
        }
        if let Some(reason) = execution.skip_reason {
            cases.borrow_mut().skipped = Some(reason.to_string());
            return Err(TestCaseError::fail("skipped"));
        }
        HitMaps::merge_opt(&mut cases.borrow_mut().coverage, execution.line_coverage.take());
        if !execution.result.status || execution.assertion_failed {
            cases.borrow_mut().logs = execution.logs;
            let reason = if execution.assertion_failed {
                "assertion failed".to_string()
            } else {
                decoder
                    .maybe_decode(&execution.result.output, None)
                    .unwrap_or_else(|| format!("{:?}", execution.result.stop))
            };
            return Err(TestCaseError::fail(reason));
        }
        let stipend = intrinsic_gas(
            &env.version,
            CALLER,
            alloy_primitives::TxKind::Call(runner.address()),
            &calldata,
            0,
            0,
            U256::ZERO,
        );
        let gas = execution.result.tx_gas_used();
        let mut cases = cases.borrow_mut();
        cases.first.get_or_insert(FuzzCase { gas, stipend });
        cases.gas.push((gas, stipend));
        Ok(())
    });
    let cases = cases.into_inner();
    let mut campaign = FuzzTestResult {
        success: outcome.is_ok(),
        first_case: cases.first.unwrap_or_default(),
        gas_by_case: cases.gas,
        logs: cases.logs,
        line_coverage: cases.coverage,
        ..Default::default()
    };
    match outcome {
        Ok(()) => {}
        Err(TestError::Fail(reason, calldata)) => {
            if let Some(skip) = cases.skipped {
                campaign.skipped = true;
                campaign.reason = Some(skip);
            } else {
                campaign.reason = Some(reason.to_string());
                let args = function.abi_decode_input(&calldata[4..]).unwrap_or_default();
                campaign.counterexample = Some(CounterExample::Single(
                    BaseCounterExample::from_fuzz_call(calldata, args, None),
                ));
            }
        }
        Err(TestError::Abort(reason)) => campaign.reason = Some(reason.to_string()),
    }
    let mut result = TestResult::default();
    result.fuzz_result(campaign);
    result.merge_coverages(runner.setup_coverage().cloned());
    result.duration = start.elapsed();
    Ok(result)
}
