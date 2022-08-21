//! Test setup

use crate::test_helpers::{COMPILED, COMPILED_WITH_LIBS, EVM_OPTS, LIBS_PROJECT, PROJECT};
use forge::{result::SuiteResult, MultiContractRunner, MultiContractRunnerBuilder, TestOptions};
use foundry_config::{Config, RpcEndpoint, RpcEndpoints};
use foundry_evm::{decode::decode_console_logs, executor::inspector::CheatsConfig};
use std::collections::BTreeMap;

pub static TEST_OPTS: TestOptions = TestOptions {
    fuzz_runs: 256,
    fuzz_max_local_rejects: 1024,
    fuzz_max_global_rejects: 65536,
    fuzz_seed: None,
    invariant_runs: 256,
    invariant_depth: 15,
    invariant_fail_on_revert: false,
    invariant_call_override: false,
};

/// Builds a base runner
pub fn base_runner() -> MultiContractRunnerBuilder {
    MultiContractRunnerBuilder::default().sender(EVM_OPTS.sender)
}

/// Builds a non-tracing runner
pub fn runner() -> MultiContractRunner {
    let mut config = Config::with_root(PROJECT.root());
    config.rpc_endpoints = rpc_endpoints();
    config.allow_paths.push(env!("CARGO_MANIFEST_DIR").into());

    base_runner()
        .with_cheats_config(CheatsConfig::new(&config, &EVM_OPTS))
        .build(
            &PROJECT.paths.root,
            (*COMPILED).clone(),
            EVM_OPTS.evm_env_blocking(),
            EVM_OPTS.clone(),
        )
        .unwrap()
}

/// Builds a tracing runner
pub fn tracing_runner() -> MultiContractRunner {
    let mut opts = EVM_OPTS.clone();
    opts.verbosity = 5;
    base_runner()
        .build(&PROJECT.paths.root, (*COMPILED).clone(), EVM_OPTS.evm_env_blocking(), opts)
        .unwrap()
}

// Builds a runner that runs against forked state
pub fn forked_runner(rpc: &str) -> MultiContractRunner {
    let mut opts = EVM_OPTS.clone();

    opts.env.chain_id = None; // clear chain id so the correct one gets fetched from the RPC
    opts.fork_url = Some(rpc.to_string());

    let env = opts.evm_env_blocking();
    let fork = opts.get_fork(&Default::default(), env.clone());

    base_runner()
        .with_fork(fork)
        .build(&LIBS_PROJECT.paths.root, (*COMPILED_WITH_LIBS).clone(), env, opts)
        .unwrap()
}

/// the RPC endpoints used during tests
pub fn rpc_endpoints() -> RpcEndpoints {
    RpcEndpoints::new([
        (
            "rpcAlias",
            RpcEndpoint::Url(
                "https://eth-mainnet.alchemyapi.io/v2/Lc7oIGYeL_QvInzI0Wiu_pOZZDEKBrdf".to_string(),
            ),
        ),
        ("rpcEnvAlias", RpcEndpoint::Env("${RPC_ENV_ALIAS}".to_string())),
    ])
}

/// A helper to assert the outcome of multiple tests with helpful assert messages
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
            "We did not run the contract {}",
            contract_name
        );

        assert_eq!(
            actuals[*contract_name].len(),
            expecteds[contract_name].len(),
            "We did not run as many test functions as we expected for {}",
            contract_name
        );
        for (test_name, should_pass, reason, expected_logs, expected_warning_count) in tests {
            let logs = decode_console_logs(&actuals[*contract_name].test_results[*test_name].logs);

            let warnings_count = &actuals[*contract_name].warnings.len();

            if *should_pass {
                assert!(
                    actuals[*contract_name].test_results[*test_name].success,
                    "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                    test_name,
                    actuals[*contract_name].test_results[*test_name].reason,
                    logs.join("\n")
                );
            } else {
                assert!(
                    !actuals[*contract_name].test_results[*test_name].success,
                    "Test {} did not fail as expected.\nLogs:\n{}",
                    test_name,
                    logs.join("\n")
                );
                assert_eq!(
                    actuals[*contract_name].test_results[*test_name].reason, *reason,
                    "Failure reason for test {} did not match what we expected.",
                    test_name
                );
            }

            if let Some(expected_logs) = expected_logs {
                assert!(
                    logs.iter().eq(expected_logs.iter()),
                    "Logs did not match for test {}.\nExpected:\n{}\n\nGot:\n{}",
                    test_name,
                    expected_logs.join("\n"),
                    logs.join("\n")
                );
            }

            if let Some(expected_warning_count) = expected_warning_count {
                assert_eq!(
                    warnings_count, expected_warning_count,
                    "Test {} did not pass as expected. Expected:\n{}Got:\n{}",
                    test_name, expected_warning_count, warnings_count
                );
            }
        }
    }
}
