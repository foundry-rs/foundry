//! Ethereum test contract execution on evm2.

use crate::{
    TestFilter,
    result::{SuiteResult, TestKind, TestResult, TestStatus},
    runner::inline_config_for,
    test_contract::{LibraryDeployment, PreparedTestArtifacts, TestContract},
    test_matcher::{TestFunctionMatcher, is_generated_symbolic_regression_contract},
};
use alloy_primitives::{Address, Bytes, KECCAK256_EMPTY, Log, U256, keccak256};
use evm2::{
    TxResult,
    ethereum::intrinsic_gas,
    evm::{Database, EmptyDB},
};
use eyre::{Result, ensure};
use foundry_cheatcodes::{CheatsConfig, ethereum::CheatcodeAccessMode};
use foundry_common::{LIBRARY_DEPLOYER, TestFunctionExt, TestFunctionKind};
use foundry_config::{Config, InlineConfig};
use foundry_evm::{
    core::{
        constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, GLOBAL_FAIL_SLOT},
        decode::{RevertDecoder, SkipReason},
        ethereum::{EthereumEnv, LocalState},
    },
    ethereum::{EthereumExecutor, EthereumInspectorStack},
    opts::EvmOpts,
};
use std::{collections::BTreeMap, sync::Arc, time::Instant};

mod fuzz;

/// Linked local test suites executed with Ethereum evm2 state.
pub(crate) struct EthereumMultiContractRunner<D: Database + Clone = EmptyDB> {
    pub prepared: PreparedTestArtifacts,
    config: Arc<Config>,
    inline_config: Arc<InlineConfig>,
    opts: EvmOpts,
    env: EthereumEnv,
    state: LocalState<D>,
}

impl<D: Database + Clone + 'static> EthereumMultiContractRunner<D> {
    /// Creates a runner from Forge's linked artifacts and Ethereum execution state.
    pub(crate) const fn new(
        prepared: PreparedTestArtifacts,
        config: Arc<Config>,
        inline_config: Arc<InlineConfig>,
        opts: EvmOpts,
        env: EthereumEnv,
        state: LocalState<D>,
    ) -> Self {
        Self { prepared, config, inline_config, opts, env, state }
    }

    /// Runs matching unit tests, cloning post-setup state for each test.
    pub(crate) fn run(&self, filter: &dyn TestFilter) -> Result<BTreeMap<String, SuiteResult>> {
        let matcher = TestFunctionMatcher::new(&self.config, &self.inline_config, None);
        let mut suites = BTreeMap::new();
        for (id, contract) in &self.prepared.contracts {
            if !matcher.matches_contract(filter, id, &contract.abi) {
                continue;
            }
            let functions =
                matcher.matching_test_functions(filter, id, &contract.abi).collect::<Vec<_>>();
            let suite_start = Instant::now();
            let contract_config =
                inline_config_for(&self.config, &self.inline_config, &id.identifier(), None)?;
            let cheats = Arc::new(CheatsConfig::new(
                &contract_config,
                self.opts.clone(),
                Some(self.prepared.known_contracts.clone()),
                Some(id.clone()),
                false,
            ));
            let runner = match EthereumContractRunner::prepare(
                contract,
                self.env,
                self.state.clone(),
                EthereumTestConfig {
                    cheats,
                    access_mode: CheatcodeAccessMode::Local,
                    sender: self.opts.sender,
                    initial_balance: self.opts.initial_balance,
                    legacy_assertions: contract_config.legacy_assertions,
                    revert_decoder: &self.prepared.revert_decoder,
                    libraries: &self.prepared.libs_to_deploy,
                    library_deployment: self.prepared.library_deployment,
                },
            ) {
                Ok(runner) => runner,
                Err(failure) => {
                    let mut result = TestResult::fail(failure.reason);
                    result.logs = failure.logs;
                    if failure.skipped {
                        result.status = TestStatus::Skipped;
                    }
                    suites.insert(
                        id.identifier(),
                        SuiteResult::new(
                            suite_start.elapsed(),
                            [(failure.name.to_string(), result)].into(),
                            Vec::new(),
                        ),
                    );
                    continue;
                }
            };
            let deprecated = functions
                .iter()
                .filter(|function| function.test_function_kind().is_any_test_fail())
                .map(|function| {
                    (
                        function.signature(),
                        TestResult::fail("`testFail*` has been removed. Consider changing to test_Revert[If|When]_Condition and expecting a revert".to_string()),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            if !deprecated.is_empty() {
                suites.insert(
                    id.identifier(),
                    SuiteResult::new(suite_start.elapsed(), deprecated, Vec::new()),
                );
                continue;
            }
            let mut tests = BTreeMap::new();
            for function in functions {
                let kind = matcher.test_function_kind(
                    &id.identifier(),
                    function,
                    is_generated_symbolic_regression_contract(&contract.abi),
                );
                if matches!(kind, TestFunctionKind::FuzzTest { .. }) {
                    let function_config = inline_config_for(
                        &self.config,
                        &self.inline_config,
                        &id.identifier(),
                        Some(function),
                    )?;
                    tests.insert(
                        function.signature(),
                        fuzz::run(
                            &runner,
                            function,
                            &function_config.fuzz,
                            &self.prepared.revert_decoder,
                            &self.env,
                        )?,
                    );
                    continue;
                }
                if !matches!(kind, TestFunctionKind::UnitTest { .. }) {
                    eyre::bail!(
                        "Ethereum evm2 execution does not yet support {} tests",
                        kind.name()
                    );
                }
                let start = Instant::now();
                let input: Bytes = function.selector().to_vec().into();
                let execution = runner.run_test(input.clone(), U256::ZERO)?;
                let result = &execution.result;
                let raw_success = result.status && !execution.assertion_failed;
                let success = raw_success;
                let stipend = intrinsic_gas(
                    &self.env.version,
                    CALLER,
                    alloy_primitives::TxKind::Call(runner.address()),
                    &input,
                    0,
                    0,
                    U256::ZERO,
                );
                let mut test_result = TestResult {
                    status: if success { TestStatus::Success } else { TestStatus::Failure },
                    reason: (!success).then(|| {
                        if execution.assertion_failed {
                            "assertion failed".to_string()
                        } else {
                            self.prepared
                                .revert_decoder
                                .maybe_decode(&result.output, None)
                                .unwrap_or_else(|| format!("{:?}", result.stop))
                        }
                    }),
                    kind: TestKind::Unit { gas: result.tx_gas_used().saturating_sub(stipend) },
                    logs: execution.logs,
                    duration: start.elapsed(),
                    ..Default::default()
                };
                if let Some(reason) = execution.skip_reason {
                    test_result.single_skip(reason);
                }
                tests.insert(function.signature(), test_result);
            }
            suites.insert(
                id.identifier(),
                SuiteResult::new(suite_start.elapsed(), tests, Vec::new()),
            );
        }
        Ok(suites)
    }
}

/// A deployed Ethereum test contract and the accepted state after `setUp`.
#[derive(Clone, Debug)]
pub(crate) struct EthereumContractRunner<D: Database + Clone = EmptyDB> {
    executor: EthereumExecutor<D, EthereumInspectorStack>,
    address: Address,
    legacy_assertions: bool,
    setup_logs: Vec<Log>,
}

#[derive(Debug)]
pub(crate) struct EthereumSetupFailure {
    name: &'static str,
    reason: String,
    skipped: bool,
    logs: Vec<Log>,
}

pub(crate) struct EthereumTestConfig<'a> {
    cheats: Arc<CheatsConfig>,
    access_mode: CheatcodeAccessMode,
    sender: Address,
    initial_balance: U256,
    legacy_assertions: bool,
    revert_decoder: &'a RevertDecoder,
    libraries: &'a [Bytes],
    library_deployment: LibraryDeployment,
}

/// One test transaction and the assertion status observed after it commits.
#[derive(Debug)]
pub(crate) struct EthereumTestExecution {
    pub result: TxResult,
    pub logs: Vec<Log>,
    pub assertion_failed: bool,
    pub skip_reason: Option<SkipReason>,
}

impl<D: Database + Clone + 'static> EthereumContractRunner<D> {
    /// Deploys one linked test contract and executes its optional `setUp()` function.
    pub(crate) fn prepare(
        contract: &TestContract,
        env: EthereumEnv,
        mut state: LocalState<D>,
        config: EthereumTestConfig<'_>,
    ) -> std::result::Result<Self, EthereumSetupFailure> {
        let EthereumTestConfig {
            cheats,
            access_mode,
            sender,
            initial_balance,
            legacy_assertions,
            revert_decoder,
            libraries,
            library_deployment,
        } = config;
        let address = (|| -> Result<_> {
            state.set_balance(sender, U256::MAX)?;
            state.set_nonce(sender, 1)?;
            state.set_balance(CALLER, U256::MAX)?;
            state.set_balance(LIBRARY_DEPLOYER, U256::MAX)?;
            let address = sender.create(1);
            state.set_balance(address, initial_balance)?;
            Ok(address)
        })()
        .map_err(|error| EthereumSetupFailure {
            name: "constructor()",
            reason: error.to_string(),
            skipped: false,
            logs: Vec::new(),
        })?;
        let mut executor = EthereumExecutor::new_foundry(env, state, cheats, access_mode);
        let deployment = (|| -> Result<()> {
            if let LibraryDeployment::Create2 { deployer, .. } = library_deployment
                && deployer == DEFAULT_CREATE2_DEPLOYER
            {
                executor.deploy_create2_deployer()?;
            }
            match library_deployment {
                LibraryDeployment::Nonce => {
                    for (nonce, code) in libraries.iter().enumerate() {
                        let expected = LIBRARY_DEPLOYER.create(nonce as u64);
                        let deployed =
                            executor.deploy(LIBRARY_DEPLOYER, code.clone(), U256::ZERO)?;
                        ensure!(
                            deployed.status && deployed.created_address == Some(expected),
                            "library deployment failed at {expected}: {:?}",
                            deployed.stop
                        );
                    }
                }
                LibraryDeployment::Create2 { deployer, salt } => {
                    for code in libraries {
                        let expected = deployer.create2_from_code(salt, code);
                        let installed = Database::get_account(executor.state_mut(), &expected)?
                            .is_some_and(|account| {
                                !account.code_hash.is_zero() && account.code_hash != KECCAK256_EMPTY
                            });
                        if installed {
                            continue;
                        }
                        let mut calldata = Vec::with_capacity(32 + code.len());
                        calldata.extend_from_slice(salt.as_slice());
                        calldata.extend_from_slice(code);
                        let deployed = executor.transact_raw(
                            LIBRARY_DEPLOYER,
                            deployer,
                            calldata.into(),
                            U256::ZERO,
                        )?;
                        ensure!(
                            deployed.status,
                            "CREATE2 library deployment failed: {:?}",
                            deployed.stop
                        );
                        let installed = Database::get_account(executor.state_mut(), &expected)?
                            .is_some_and(|account| {
                                !account.code_hash.is_zero() && account.code_hash != KECCAK256_EMPTY
                            });
                        ensure!(installed, "CREATE2 library has no code at {expected}");
                    }
                }
            }
            let deployed = executor.deploy(sender, contract.bytecode.clone(), U256::ZERO)?;
            ensure!(
                deployed.status && deployed.created_address == Some(address),
                "{}",
                revert_decoder
                    .maybe_decode(&deployed.output, None)
                    .unwrap_or_else(|| format!("{:?}", deployed.stop))
            );
            executor.state_mut().set_balance(sender, initial_balance)?;
            executor.state_mut().set_balance(CALLER, initial_balance)?;
            executor.state_mut().set_balance(LIBRARY_DEPLOYER, initial_balance)?;
            if matches!(library_deployment, LibraryDeployment::Nonce) {
                executor.deploy_create2_deployer()?;
            }
            Ok(())
        })();
        deployment.map_err(|error| EthereumSetupFailure {
            name: "constructor()",
            reason: error.to_string(),
            skipped: false,
            logs: executor.inspector_mut().take_logs(),
        })?;
        let mut setup_logs = executor.inspector_mut().take_logs();
        executor.inspector_mut().cheatcodes_mut().take_skip_payloads();
        if let Some(setup) =
            contract.abi.functions().find(|f| f.name == "setUp" && f.inputs.is_empty())
        {
            let result = executor
                .transact_raw(CALLER, address, setup.selector().to_vec().into(), U256::ZERO)
                .map_err(|error| EthereumSetupFailure {
                    name: "setUp()",
                    reason: error.to_string(),
                    skipped: false,
                    logs: setup_logs.clone(),
                })?;
            let skip_reason = Self::take_skip_reason(&mut executor, &result);
            let assertion_failed = result.status
                && Self::assertion_failed(&mut executor, address, legacy_assertions).map_err(
                    |error| EthereumSetupFailure {
                        name: "setUp()",
                        reason: error.to_string(),
                        skipped: false,
                        logs: setup_logs.clone(),
                    },
                )?;
            setup_logs.extend(executor.inspector_mut().take_logs());
            if !result.status || assertion_failed {
                return Err(EthereumSetupFailure {
                    name: "setUp()",
                    reason: if let Some(reason) = &skip_reason {
                        reason.to_string()
                    } else if assertion_failed {
                        "assertion failed".to_string()
                    } else {
                        revert_decoder
                            .maybe_decode(&result.output, None)
                            .unwrap_or_else(|| format!("{:?}", result.stop))
                    },
                    skipped: skip_reason.is_some(),
                    logs: setup_logs,
                });
            }
        }
        Ok(Self { executor, address, legacy_assertions, setup_logs })
    }

    /// Returns the deployed test contract address.
    pub(crate) const fn address(&self) -> Address {
        self.address
    }

    /// Executes one test against an isolated copy of the state after `setUp`.
    pub(crate) fn run_test(&self, calldata: Bytes, value: U256) -> Result<EthereumTestExecution> {
        let mut executor = self.executor.clone();
        let result = executor.transact_raw(CALLER, self.address, calldata, value)?;
        let mut logs = self.setup_logs.clone();
        logs.extend(executor.inspector_mut().take_logs());
        let skip_reason = Self::take_skip_reason(&mut executor, &result);
        let assertion_failed = result.status
            && Self::assertion_failed(&mut executor, self.address, self.legacy_assertions)?;
        Ok(EthereumTestExecution { result, logs, assertion_failed, skip_reason })
    }

    fn take_skip_reason(
        executor: &mut EthereumExecutor<D, EthereumInspectorStack>,
        result: &TxResult,
    ) -> Option<SkipReason> {
        let payloads = executor.inspector_mut().cheatcodes_mut().take_skip_payloads();
        (!result.status && payloads.contains(&result.output))
            .then(|| SkipReason::decode(&result.output))
            .flatten()
    }

    fn assertion_failed(
        executor: &mut EthereumExecutor<D, EthereumInspectorStack>,
        address: Address,
        legacy_assertions: bool,
    ) -> Result<bool> {
        let global_failed =
            !Database::get_storage(executor.state_mut(), &CHEATCODE_ADDRESS, &GLOBAL_FAIL_SLOT)?
                .is_zero();
        let legacy_failed = if legacy_assertions {
            let selector = &keccak256("failed()")[..4];
            executor
                .call_raw(CALLER, address, selector.to_vec().into(), U256::ZERO)
                .ok()
                .filter(|call| call.status && call.output.len() == 32)
                .is_some_and(|call| !U256::from_be_slice(&call.output).is_zero())
        } else {
            false
        };
        Ok(global_failed || legacy_failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_abi::{Function, JsonAbi};
    use evm2::SpecId;

    #[test]
    fn setup_state_is_inherited_but_test_mutations_are_isolated() {
        let setup = Function::parse("setUp()").unwrap();
        let test = Function::parse("testIncrement()").unwrap();
        let mut abi = JsonAbi::new();
        abi.functions.entry(setup.name.clone()).or_default().push(setup.clone());
        abi.functions.entry(test.name.clone()).or_default().push(test.clone());

        let mut runtime = vec![0x63];
        runtime.extend_from_slice(setup.selector().as_slice());
        runtime.extend_from_slice(&[0x5f, 0x35, 0x60, 0xe0, 0x1c, 0x14, 0x60, 0, 0x57]);
        let setup_jump = runtime.len() - 2;
        runtime.extend_from_slice(&[
            0x5f, 0x54, 0x60, 0x01, 0x01, 0x80, 0x5f, 0x55, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3,
        ]);
        runtime[setup_jump] = runtime.len() as u8;
        runtime.extend_from_slice(&[0x5b, 0x60, 0x01, 0x5f, 0x55, 0x00]);

        let mut bytecode = vec![0x60, runtime.len() as u8, 0x60, 0x0a, 0x5f, 0x39];
        bytecode.extend_from_slice(&[0x60, runtime.len() as u8, 0x5f, 0xf3]);
        bytecode.extend_from_slice(&runtime);
        let contract =
            TestContract { abi, bytecode: bytecode.into(), library_addresses: Default::default() };
        let sender = Address::with_last_byte(0xa);
        let mut opts = EvmOpts::default();
        opts.env.gas_limit = 30_000_000.into();
        opts.memory_limit = 128 * 1024 * 1024;
        let env = EthereumEnv::local(SpecId::CANCUN, &opts);
        let runner = EthereumContractRunner::prepare(
            &contract,
            env,
            LocalState::default(),
            EthereumTestConfig {
                cheats: Arc::default(),
                access_mode: CheatcodeAccessMode::Local,
                sender,
                initial_balance: U256::from(100),
                legacy_assertions: false,
                revert_decoder: &RevertDecoder::new(),
                libraries: &[],
                library_deployment: LibraryDeployment::Nonce,
            },
        )
        .unwrap();
        assert_eq!(runner.address(), sender.create(1));
        for _ in 0..2 {
            let execution = runner.run_test(test.selector().to_vec().into(), U256::ZERO).unwrap();
            assert!(execution.result.status, "{:?}", execution.result);
            assert_eq!(U256::from_be_slice(&execution.result.output), U256::from(2));
            assert!(execution.logs.is_empty());
            assert!(!execution.assertion_failed);
        }
    }
}
