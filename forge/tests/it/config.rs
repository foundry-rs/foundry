//! Test setup

use crate::test_helpers::{
    filter::Filter, COMPILED, COMPILED_WITH_LIBS, EVM_OPTS, LIBS_PROJECT, PROJECT,
};
use ethers::solc::EvmVersion;
use forge::{result::SuiteResult, MultiContractRunner, MultiContractRunnerBuilder, TestOptions};
use foundry_config::{
    fs_permissions::PathPermission, Config, FsPermissions, FuzzConfig, FuzzDictionaryConfig,
    InvariantConfig, RpcEndpoint, RpcEndpoints,
};
use foundry_evm::{
    decode::decode_console_logs, executor::inspector::CheatsConfig, revm::primitives::SpecId,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// How to execute a a test run
pub struct TestConfig {
    pub runner: MultiContractRunner,
    pub should_fail: bool,
    pub filter: Filter,
    pub opts: TestOptions,
}

// === impl TestConfig ===

impl TestConfig {
    pub fn new(runner: MultiContractRunner) -> Self {
        Self::with_filter(runner, Filter::matches_all())
    }

    pub fn with_filter(runner: MultiContractRunner, filter: Filter) -> Self {
        Self { runner, should_fail: false, filter, opts: TEST_OPTS }
    }

    pub fn filter(filter: Filter) -> Self {
        Self { filter, ..Default::default() }
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
        self.runner.test(&self.filter, None, self.opts).unwrap()
    }

    #[track_caller]
    pub fn run(&mut self) {
        self.try_run().unwrap()
    }

    /// Executes the test case
    ///
    /// Returns an error if
    ///    * filter matched 0 test cases
    ///    * a test results deviates from the configured `should_fail` setting
    pub fn try_run(&mut self) -> eyre::Result<()> {
        let suite_result = self.runner.test(&self.filter, None, self.opts).unwrap();
        if suite_result.is_empty() {
            eyre::bail!("empty test result");
        }
        for (_, SuiteResult { test_results, .. }) in suite_result {
            for (test_name, result) in test_results {
                if self.should_fail != !result.success {
                    let logs = decode_console_logs(&result.logs);
                    let outcome = if self.should_fail { "fail" } else { "pass" };

                    eyre::bail!(
                        "Test {} did not {} as expected.\nReason: {:?}\nLogs:\n{}",
                        test_name,
                        outcome,
                        result.reason,
                        logs.join("\n")
                    )
                }
            }
        }

        Ok(())
    }
}

impl Default for TestConfig {
    fn default() -> Self {
        TestConfig::new(runner())
    }
}

pub static TEST_OPTS: TestOptions = TestOptions {
    fuzz: FuzzConfig {
        runs: 256,
        max_test_rejects: 65536,
        seed: None,
        dictionary: FuzzDictionaryConfig {
            include_storage: true,
            include_push_bytes: true,
            dictionary_weight: 40,
            max_fuzz_dictionary_addresses: 10_000,
            max_fuzz_dictionary_values: 10_000,
        },
    },
    invariant: InvariantConfig {
        runs: 256,
        depth: 15,
        fail_on_revert: false,
        call_override: false,
        dictionary: FuzzDictionaryConfig {
            dictionary_weight: 80,
            include_storage: true,
            include_push_bytes: true,
            max_fuzz_dictionary_addresses: 10_000,
            max_fuzz_dictionary_values: 10_000,
        },
    },
};

pub fn manifest_root() -> PathBuf {
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // need to check here where we're executing the test from, if in `forge` we need to also allow
    // `testdata`
    if root.ends_with("forge") {
        root = root.parent().unwrap();
    }
    root.to_path_buf()
}

/// Builds a base runner
pub fn base_runner() -> MultiContractRunnerBuilder {
    MultiContractRunnerBuilder::default().sender(EVM_OPTS.sender)
}

/// Builds a non-tracing runner
pub fn runner() -> MultiContractRunner {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read_write(manifest_root())]);
    runner_with_config(config)
}

/// Builds a non-tracing runner
pub fn runner_with_config(mut config: Config) -> MultiContractRunner {
    config.rpc_endpoints = rpc_endpoints();
    config.allow_paths.push(manifest_root());

    base_runner()
        .with_cheats_config(CheatsConfig::new(&config, &EVM_OPTS))
        .sender(config.sender)
        .build(
            &PROJECT.paths.root,
            (*COMPILED).clone(),
            EVM_OPTS.evm_env_blocking().unwrap(),
            EVM_OPTS.clone(),
        )
        .unwrap()
}

/// Builds a tracing runner
pub fn tracing_runner() -> MultiContractRunner {
    let mut opts = EVM_OPTS.clone();
    opts.verbosity = 5;
    base_runner()
        .build(&PROJECT.paths.root, (*COMPILED).clone(), EVM_OPTS.evm_env_blocking().unwrap(), opts)
        .unwrap()
}

// Builds a runner that runs against forked state
pub fn forked_runner(rpc: &str) -> MultiContractRunner {
    let mut opts = EVM_OPTS.clone();

    opts.env.chain_id = None; // clear chain id so the correct one gets fetched from the RPC
    opts.fork_url = Some(rpc.to_string());

    let env = opts.evm_env_blocking().unwrap();
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
            let logs = &actuals[*contract_name].test_results[*test_name].decoded_logs;

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
                    "Failure reason for test {test_name} did not match what we expected."
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
                    "Test {test_name} did not pass as expected. Expected:\n{expected_warning_count}Got:\n{warnings_count}"
                );
            }
        }
    }
}

/// Converts an `EvmVersion` into a `SpecId`
pub fn evm_spec(evm: &EvmVersion) -> SpecId {
    match evm {
        EvmVersion::Istanbul => SpecId::ISTANBUL,
        EvmVersion::Berlin => SpecId::BERLIN,
        EvmVersion::London => SpecId::LONDON,
        EvmVersion::Paris => SpecId::MERGE,
        EvmVersion::Shanghai => SpecId::SHANGHAI,
        _ => panic!("Unsupported EVM version"),
    }
}
