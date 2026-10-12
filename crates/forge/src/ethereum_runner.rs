//! Forge execution over the native Ethereum EVM2 executor.

use crate::{
    TestFilter,
    result::{
        InvariantFailure, InvariantPredicateResult, SuiteResult, TestKind, TestResult, TestStatus,
    },
    test_artifacts::{LibraryDeployment, PreparedTestArtifacts, prepare_artifacts},
    test_config::{TestFunctionMatcher, inline_config_for, select_invariant_campaigns},
};
use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_primitives::{Address, B256, Bytes, KECCAK256_EMPTY, TxKind, U256, keccak256};
use alloy_rpc_types::state::{AccountOverride, StateOverride};
use alloy_sol_types::SolValue;
use evm2::{
    ExecutionConfig,
    bytecode::Bytecode,
    ethereum::{TxEnvelope, intrinsic_gas},
    evm::{AccountInfo, DbResult, DynDatabase, EmptyDB, TxResult},
};
use eyre::{Result, ensure};
use foundry_cheatcodes::{CheatsConfig, ethereum::Cheatcodes};
use foundry_common::{TestFunctionExt, TestFunctionKind};
use foundry_compilers::ProjectCompileOutput;
use foundry_config::{Config, InlineConfig};
use foundry_evm::{
    core::{
        constants::{
            CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER,
            DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE, GLOBAL_FAIL_SLOT,
        },
        decode::decode_console_logs,
        ethereum::{EthereumEnv, EthereumFork, FoundryContext, FoundryEvmTypes, RpcDatabase},
    },
    ethereum::Executor,
    opts::EvmOpts,
};
use foundry_evm_coverage::HitMaps;
use std::{collections::BTreeMap, sync::Arc, time::Instant};

mod fuzz;

mod invariant;

/// Linked artifacts and policy for a native Ethereum test pass.
pub(crate) struct EthereumRunner {
    pub artifacts: PreparedTestArtifacts,
    config: Arc<Config>,
    inline: Arc<InlineConfig>,
    opts: EvmOpts,
    fork: Option<EthereumFork>,
    line_coverage: bool,
}

impl EthereumRunner {
    pub(crate) async fn new(
        config: Arc<Config>,
        inline: Arc<InlineConfig>,
        line_coverage: bool,
        output: &ProjectCompileOutput,
        opts: EvmOpts,
    ) -> Result<Self> {
        let mut fork = if opts.fork_url.is_some() {
            Some(EthereumFork::open(&config, &opts, None).await?)
        } else {
            None
        };
        let create2_available = match &mut fork {
            Some(fork) => fork
                .database
                .get_account(&opts.create2_deployer)?
                .is_some_and(|account| account.code_hash != KECCAK256_EMPTY),
            None => opts.create2_deployer == DEFAULT_CREATE2_DEPLOYER,
        };
        let artifacts = prepare_artifacts(
            &config,
            line_coverage,
            TestFunctionMatcher::new(&config, &inline, None),
            output,
            &opts,
            create2_available,
        )?;
        Ok(Self { artifacts, config, inline, opts, fork, line_coverage })
    }

    pub(crate) fn run(
        &self,
        filter: &dyn TestFilter,
        fuzz_only: bool,
    ) -> Result<BTreeMap<String, SuiteResult>> {
        let matcher = TestFunctionMatcher::new(&self.config, &self.inline, None);
        let mut suites = BTreeMap::new();
        for (id, contract) in &self.artifacts.contracts {
            if !matcher.matches_contract(filter, id, &contract.abi) {
                continue;
            }
            let functions = matcher
                .matching_test_functions(filter, id, &contract.abi)
                .filter(|function| {
                    !fuzz_only || function.is_fuzz_test() || function.is_invariant_test()
                })
                .collect::<Vec<_>>();
            if functions.is_empty() {
                continue;
            }
            let timer = Instant::now();
            let config = inline_config_for(&self.config, &self.inline, &id.identifier(), None)?;
            let mut env = EthereumEnv::local_from_config(&config, &self.opts)?;
            let database = match &self.fork {
                Some(fork) => {
                    // Local Ethereum fork tests retain the configured execution hardfork.
                    env.block = fork.env.block;
                    env.version.chain_id = fork.env.version.chain_id;
                    env.gas_price = fork.env.gas_price;
                    Database::Fork(fork.database.clone().with_spec(env.spec))
                }
                None => Database::Local(EmptyDB::default()),
            };
            // Foundry synthetic test transactions use the configured gas limit for both the
            // transaction and its block, including fork execution.
            env.block.gas_limit = U256::from(self.opts.gas_limit());
            let mut executor = Executor::new(
                database,
                env.spec,
                ExecutionConfig::for_spec_and_version(env.spec, env.version),
                env.block,
            );
            let mut overrides = [
                (
                    self.opts.sender,
                    AccountOverride {
                        balance: Some(U256::MAX),
                        nonce: Some(1),
                        ..Default::default()
                    },
                ),
                (
                    foundry_common::LIBRARY_DEPLOYER,
                    AccountOverride {
                        balance: Some(U256::MAX),
                        nonce: Some(0),
                        ..Default::default()
                    },
                ),
                (
                    CHEATCODE_ADDRESS,
                    AccountOverride { code: Some(Bytes::from_static(&[0])), ..Default::default() },
                ),
            ]
            .into_iter()
            .collect::<StateOverride>();
            if self.fork.is_none() {
                overrides.insert(
                    DEFAULT_CREATE2_DEPLOYER,
                    AccountOverride {
                        code: Some(Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE)),
                        ..Default::default()
                    },
                );
            }
            executor.apply_state_overrides(overrides)?;
            executor.set_context(FoundryContext {
                gas_price_override: Some(env.gas_price),
                basefee_override: Some(env.block.basefee),
                ..Default::default()
            });
            let mut opts = self.opts.clone();
            opts.always_use_create_2_factory = config.always_use_create_2_factory;
            let mut cheats = Cheatcodes::new(Arc::new(CheatsConfig::new(
                &config,
                opts,
                Some(self.artifacts.known_contracts.clone()),
                Some(id.clone()),
                false,
            )));
            cheats.line_coverage = self.line_coverage.then(Default::default);
            if let Some(fork) = &self.fork {
                cheats.initialize_fork(fork);
            }
            for code in &self.artifacts.libs_to_deploy {
                let (caller, to, input) = match self.artifacts.library_deployment {
                    LibraryDeployment::Nonce => {
                        (foundry_common::LIBRARY_DEPLOYER, TxKind::Create, code.clone())
                    }
                    LibraryDeployment::Create2 { deployer, salt } => (
                        self.opts.sender,
                        TxKind::Call(deployer),
                        Bytes::from([salt.as_slice(), code.as_ref()].concat()),
                    ),
                };
                let (result, next) =
                    executor.inspect_transact(&self.tx(&executor, caller, to, input)?, cheats);
                cheats = next;
                ensure!(result?.status, "library deployment failed");
            }
            let (deployment, next) = executor.inspect_transact(
                &self.tx(&executor, self.opts.sender, TxKind::Create, contract.bytecode.clone())?,
                cheats,
            );
            cheats = next;
            let deployment = deployment?;
            let assertion_failed = executor.storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)?
                != U256::ZERO
                || cheats.snapshot_failure;
            if !deployment.status || assertion_failed {
                let result = self.outcome(deployment, assertion_failed, 0, cheats, timer.elapsed());
                suites.insert(
                    id.identifier(),
                    SuiteResult::new(
                        timer.elapsed(),
                        BTreeMap::from([("constructor()".to_owned(), result)]),
                        Vec::new(),
                    ),
                );
                continue;
            }
            let address = deployment.created_address.expect("successful deployment has no address");
            executor.set_balance(address, self.opts.initial_balance)?;
            if let Some(setup) =
                contract.abi.functions.get("setUp").and_then(|functions| {
                    functions.iter().find(|function| function.inputs.is_empty())
                })
            {
                let input = Bytes::copy_from_slice(setup.selector().as_slice());
                let (result, next) = executor.inspect_transact(
                    &self.tx(&executor, self.opts.sender, TxKind::Call(address), input.clone())?,
                    cheats,
                );
                cheats = next;
                let result = result?;
                let assertion_failed = executor.storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)?
                    != U256::ZERO
                    || cheats.snapshot_failure;
                if !result.status || assertion_failed {
                    let result = self.outcome(
                        result,
                        assertion_failed,
                        self.stipend(&env, address, &input),
                        cheats,
                        timer.elapsed(),
                    );
                    suites.insert(
                        id.identifier(),
                        SuiteResult::new(
                            timer.elapsed(),
                            BTreeMap::from([("setUp()".to_owned(), result)]),
                            Vec::new(),
                        ),
                    );
                    continue;
                }
            }
            let setup_coverage =
                cheats.line_coverage.as_mut().map(|coverage| std::mem::take(coverage).finish());
            let mut results = BTreeMap::new();
            let fixtures = functions
                .iter()
                .any(|function| !function.inputs.is_empty())
                .then(|| self.fuzz_fixtures(&executor, &cheats, address, &contract.abi))
                .transpose()?;
            let all_invariants =
                contract.abi.functions().filter(|f| f.is_invariant_test()).collect::<Vec<_>>();
            let campaigns = select_invariant_campaigns(
                &all_invariants,
                &functions,
                &config,
                &self.inline,
                &id.identifier(),
            );
            trace!(contract = %id.identifier(), campaigns = campaigns.anchor_count(), shared_boolean_namespace = campaigns.shared_boolean_namespace, "selected native invariant campaigns");
            for function in functions.iter().copied() {
                ensure!(
                    matches!(
                        function.test_function_kind(),
                        TestFunctionKind::UnitTest { .. }
                            | TestFunctionKind::FuzzTest { .. }
                            | TestFunctionKind::InvariantTest
                    ),
                    "native EVM2 table execution is not migrated yet: {}",
                    function.signature()
                );
                let test_config =
                    inline_config_for(&config, &self.inline, &id.identifier(), Some(function))?;
                let mut test_cheats = cheats.clone();
                if test_config != config {
                    test_cheats.reconfigure(&test_config);
                }
                if function.is_invariant_test() {
                    let predicates = if function.outputs.is_empty() && campaigns.merge_boolean_suite
                    {
                        if campaigns.boolean_suite_anchor != Some(function) {
                            continue;
                        }
                        campaigns.matched_boolean_invariant_fns.as_slice()
                    } else {
                        std::slice::from_ref(&function)
                    };
                    let predicate_results = self.invariant(
                        &executor,
                        &test_cheats,
                        address,
                        predicates,
                        &test_config,
                        &id.identifier(),
                        &contract.abi,
                    )?;
                    let mut result = predicate_results[0].clone();
                    if predicates.len() > 1 {
                        result.invariant_count = Some(predicates.len());
                        if let TestKind::Invariant { failed_corpus_replays, .. } = &mut result.kind
                        {
                            *failed_corpus_replays = predicate_results
                                .iter()
                                .map(|result| match &result.kind {
                                    TestKind::Invariant { failed_corpus_replays, .. } => {
                                        *failed_corpus_replays
                                    }
                                    _ => 0,
                                })
                                .sum();
                        }
                        result.invariant_predicate_results = predicates
                            .iter()
                            .zip(&predicate_results)
                            .map(|(predicate, result)| InvariantPredicateResult {
                                name: predicate.name.clone(),
                                status: if result.invariant_handler_failures.is_empty() {
                                    result.status
                                } else {
                                    TestStatus::Success
                                },
                                reason: result
                                    .invariant_handler_failures
                                    .is_empty()
                                    .then(|| result.reason.clone())
                                    .flatten(),
                            })
                            .collect();
                        result.invariant_failures = predicates
                            .iter()
                            .zip(&predicate_results)
                            .filter(|(_, result)| {
                                result.status.is_failure()
                                    && result.invariant_handler_failures.is_empty()
                            })
                            .map(|(predicate, result)| InvariantFailure::Predicate {
                                name: predicate.name.clone(),
                                reason: result.reason.clone().unwrap_or_default(),
                                counterexample: result.counterexample.clone(),
                                artifact: None,
                                minimization: None,
                                persisted_path: test_config
                                    .invariant
                                    .failure_persist_dir
                                    .as_ref()
                                    .map(|dir| {
                                        dir.join("failures")
                                            .join(&id.name)
                                            .join("invariants")
                                            .join(&predicate.name)
                                    })
                                    .unwrap_or_default(),
                                is_anchor: *predicate == function,
                            })
                            .collect();
                        result.invariant_handler_failures = predicate_results
                            .iter()
                            .find(|result| !result.invariant_handler_failures.is_empty())
                            .map(|result| result.invariant_handler_failures.clone())
                            .unwrap_or_default();
                        result.status = if predicate_results.iter().any(|r| r.status.is_failure()) {
                            TestStatus::Failure
                        } else if predicate_results.iter().all(|r| r.status.is_skipped()) {
                            TestStatus::Skipped
                        } else {
                            TestStatus::Success
                        };
                        result.reason = None;
                        result.counterexample = None;
                    }
                    results.insert(function.signature(), result);
                    continue;
                }
                if !function.inputs.is_empty() {
                    let result = self.fuzz(
                        &executor,
                        &test_cheats,
                        address,
                        function,
                        fixtures.as_ref().expect("fuzz fixtures prepared"),
                        &env,
                        &test_config,
                        &id.identifier(),
                        &contract.abi,
                    )?;
                    results.insert(function.signature(), result);
                    continue;
                }
                let test_timer = Instant::now();
                let mut test_executor = executor.clone();
                let input = Bytes::copy_from_slice(function.selector().as_slice());
                let (result, observations) = test_executor.inspect_transact(
                    &self.tx(
                        &test_executor,
                        self.opts.sender,
                        TxKind::Call(address),
                        input.clone(),
                    )?,
                    test_cheats,
                );
                let result = result?;
                let assertion_failed =
                    test_executor.storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)? != U256::ZERO;
                let legacy_failed = result.status
                    && !assertion_failed
                    && !observations.snapshot_failure
                    && test_config.legacy_assertions
                    && contract.abi.functions.contains_key("failed")
                    && self.legacy_failed(&test_executor, &observations, address)?;
                let mut result = self.outcome(
                    result,
                    assertion_failed,
                    self.stipend(&env, address, &input),
                    observations,
                    test_timer.elapsed(),
                );
                // Legacy DSTest assertions can return successful EVM status while setting failed().
                if result.status.is_success() && legacy_failed {
                    result.status = TestStatus::Failure;
                }
                results.insert(function.signature(), result);
            }
            for result in results.values_mut() {
                result.merge_coverages(setup_coverage.clone());
            }
            suites.insert(id.identifier(), SuiteResult::new(timer.elapsed(), results, Vec::new()));
        }
        Ok(suites)
    }

    fn tx(
        &self,
        executor: &Executor<Database>,
        caller: Address,
        to: TxKind,
        input: Bytes,
    ) -> Result<Recovered<TxEnvelope>> {
        let nonce = executor.account_info(caller)?.map_or(0, |info| info.nonce);
        Ok(Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                to,
                input,
                nonce,
                gas_limit: self.opts.gas_limit(),
                ..Default::default()
            }),
            caller,
        ))
    }

    fn stipend(&self, env: &EthereumEnv, address: Address, input: &Bytes) -> u64 {
        intrinsic_gas(
            &env.version,
            self.opts.sender,
            TxKind::Call(address),
            input,
            0,
            0,
            U256::ZERO,
        )
    }

    fn outcome(
        &self,
        result: TxResult<FoundryEvmTypes>,
        assertion_failed: bool,
        stipend: u64,
        cheats: Cheatcodes,
        duration: std::time::Duration,
    ) -> TestResult {
        let assertion_failed = assertion_failed || cheats.snapshot_failure;
        let skip = cheats.skip_payloads.iter().find(|payload| **payload == result.output);
        let status = if skip.is_some() {
            TestStatus::Skipped
        } else if result.status && !assertion_failed {
            TestStatus::Success
        } else {
            TestStatus::Failure
        };
        let reason = status.is_failure().then(|| {
            if assertion_failed && result.status {
                "assertion failed".to_owned()
            } else if result.output.is_empty() {
                format!("EvmError: {:?}", result.stop)
            } else {
                self.artifacts.revert_decoder.decode_data(&result.output)
            }
        });
        TestResult {
            status,
            reason,
            kind: TestKind::Unit { gas: result.tx_gas_used().saturating_sub(stipend) },
            decoded_logs: decode_console_logs(&cheats.logs),
            logs: cheats.logs,
            line_coverage: cheats.line_coverage.map(|coverage| coverage.finish()),
            duration,
            ..Default::default()
        }
    }

    fn legacy_failed(
        &self,
        executor: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
    ) -> Result<bool> {
        let input = Bytes::copy_from_slice(&keccak256("failed()")[..4]);
        let (failed, _) = executor.inspect_call(
            &self.tx(executor, CALLER, TxKind::Call(address), input)?,
            cheats.clone(),
        );
        let failed = failed?;
        Ok(failed.status && bool::abi_decode(&failed.output).unwrap_or_default())
    }
}

/// Read-only backing selected once for the native test pass. Writes belong to the executor.
#[derive(Clone)]
enum Database {
    Local(EmptyDB),
    Fork(RpcDatabase),
}

impl DynDatabase for Database {
    fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
        match self {
            Self::Local(database) => database.get_account(address),
            Self::Fork(database) => database.get_account(address),
        }
    }

    fn get_code_by_hash(&mut self, hash: &B256) -> DbResult<Bytecode> {
        match self {
            Self::Local(database) => database.get_code_by_hash(hash),
            Self::Fork(database) => database.get_code_by_hash(hash),
        }
    }

    fn get_storage(&mut self, address: &Address, key: &U256) -> DbResult<U256> {
        match self {
            Self::Local(database) => database.get_storage(address, key),
            Self::Fork(database) => database.get_storage(address, key),
        }
    }

    fn get_block_hash(&mut self, number: &U256) -> DbResult<B256> {
        match self {
            Self::Local(database) => database.get_block_hash(number),
            Self::Fork(database) => database.get_block_hash(number),
        }
    }
}
