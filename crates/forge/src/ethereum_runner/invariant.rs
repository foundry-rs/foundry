//! Stateful Ethereum invariant campaigns over owned EVM2 execution state.

use super::*;
use crate::invariant_failure::{
    InvariantFailureIdentity, InvariantFailureSite as FailureSite, InvariantPersistedFailure,
};
use alloy_json_abi::{Function, JsonAbi};
use evm2::evm::{AccountChangeRef, StateChangeSink, StorageChange};
use foundry_config::{InvariantDepthMode, InvariantWorkers};
use foundry_evm_fuzz::{
    BaseCounterExample, BasicTxDetails, CounterExample,
    invariant::{
        FuzzRunIdentifiedContracts, InvariantMetrics, InvariantSettings, TargetedContract,
    },
    strategies::{EvmFuzzState, FuzzState, TxGenerator},
};
use rand::Rng;
use replay::Failure;
use std::{collections::HashMap, convert::Infallible};

mod replay;
mod targets;

impl EthereumRunner {
    #[expect(
        clippy::too_many_arguments,
        reason = "Campaign uses the prepared test and owned setup state."
    )]
    pub(super) fn invariant(
        &self,
        executor: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
        functions: &[&Function],
        config: &Config,
        contract: &str,
        abi: &JsonAbi,
    ) -> Result<Vec<TestResult>> {
        let options = &config.invariant;
        ensure!(
            !options.call_override,
            "native EVM2 invariant call overrides are not migrated yet"
        );
        ensure!(
            options.corpus.corpus_dir.is_none(),
            "native EVM2 invariant coverage corpus is not migrated yet"
        );
        ensure!(
            !matches!(options.workers, InvariantWorkers::Fixed(n) if n.get() > 1),
            "native EVM2 invariant worker sharding is not migrated yet"
        );
        ensure!(
            functions.iter().all(|function| function.outputs.is_empty()),
            "native EVM2 optimization invariants are not migrated yet"
        );
        let timer = Instant::now();
        let targets = self.invariant_targets(executor, cheats, address)?;
        let settings =
            InvariantSettings::new(&targets.contracts, &targets.senders, options.fail_on_revert);
        let failure_dir = options.failure_persist_dir.as_ref().map(|dir| {
            dir.join("failures").join(contract.split(':').next_back().unwrap_or(contract))
        });
        let mut persisted = Vec::new();
        let mut failures = Vec::new();
        for function in functions {
            let failure_file =
                failure_dir.as_ref().map(|dir| dir.join("invariants").join(&function.name));
            let saved = failure_file
                .as_ref()
                .and_then(|path| replay::persisted_failure(path, &settings))
                .or_else(|| {
                    failure_dir.as_ref().and_then(|dir| {
                        replay::persisted_failure(&dir.join(&function.name), &settings)
                    })
                });
            let replayed = if let Some(calls) = saved.as_ref().and_then(replay::failure_calls) {
                self.replay_invariant(
                    executor, cheats, address, function, config, abi, &targets, &calls,
                )?
                .filter(|failure| {
                    saved.as_ref().is_some_and(|saved| {
                        saved.assertion_failure == failure.assertion
                            && saved.execution_failure.as_ref().is_none_or(|identity| {
                                identity.site == failure.site
                                    && identity.reason == failure.result.reason
                            })
                    })
                })
            } else {
                None
            };
            persisted.push(replayed.is_some());
            failures.push(replayed);
        }
        let fixtures = self.fuzz_fixtures(executor, cheats, address, abi)?;
        let mut seed = EvmFuzzState::empty(
            &self.artifacts.library_addresses,
            options.dictionary,
            Some(&self.artifacts.invariant_literals),
        );
        let mut accounts = executor.accepted_cache().accounts.iter().collect::<Vec<_>>();
        accounts.sort_unstable_by_key(|(address, _)| **address);
        for (address, account) in accounts {
            if let Some(account) = account {
                let code = executor.code(*address)?;
                seed.seed_account(
                    *address,
                    Some((account.code_hash, code.original_byte_slice())),
                    executor
                        .accepted_cache()
                        .storage
                        .get(address)
                        .into_iter()
                        .flat_map(|s| s.slots.iter().map(|(slot, value)| (*slot, *value))),
                );
            }
        }
        let dictionary = seed.into_invariant();
        let mut random = crate::test_config::fuzzer_with_cases(
            config.fuzz.seed,
            options.runs,
            options.max_assume_rejects,
        );
        let mut completed = 0usize;
        let mut calls = 0usize;
        let mut reverts = 0usize;
        let mut metrics = HashMap::<String, InvariantMetrics>::default();
        let mut tests = functions
            .iter()
            .map(|function| self.check_invariant(executor, cheats, address, function, config, abi))
            .collect::<Result<Vec<_>>>()?;
        let mut campaign_coverage = None;
        for test in &mut tests {
            HitMaps::merge_opt(&mut campaign_coverage, test.line_coverage.take());
        }
        if tests
            .iter()
            .zip(&failures)
            .any(|(test, failure)| test.status.is_success() && failure.is_none())
        {
            'campaign: for _ in 0..options.runs {
                if options
                    .timeout
                    .is_some_and(|seconds| timer.elapsed().as_secs() >= u64::from(seconds))
                {
                    break;
                }
                completed += 1;
                let mut state = executor.clone();
                let mut inspector = cheats.clone();
                // Runtime samples survive runs; ephemeral state values follow the run reset.
                dictionary.revert();
                let contracts =
                    FuzzRunIdentifiedContracts::new(targets.contracts.clone(), targets.updatable);
                let generator = TxGenerator::invariant(
                    dictionary.clone(),
                    targets.senders.clone(),
                    contracts.clone(),
                    options.clone(),
                    fixtures.clone(),
                );
                let depth = match options.depth_mode {
                    InvariantDepthMode::Fixed => options.depth,
                    InvariantDepthMode::Random => {
                        random.rng().random_range(options.min_depth..=options.depth)
                    }
                };
                let mut sequence = Vec::new();
                let mut rejects = 0u32;
                let mut accepted = 0u32;
                while accepted < depth {
                    let mut tx = generator.next_tx(&mut random)?;
                    let (result, mut observed) =
                        self.invariant_call(&mut state, &inspector, &mut tx, Some(&dictionary))?;
                    if let Some(coverage) = &mut observed.line_coverage {
                        HitMaps::merge_opt(
                            &mut campaign_coverage,
                            Some(std::mem::take(coverage).finish()),
                        );
                    }
                    let rejected =
                        result.output.as_ref() == foundry_evm::core::constants::MAGIC_ASSUME;
                    let failed_assertion =
                        self.invariant_handler_assertion(&state, &result, &observed)?;
                    calls += 1;
                    if let Some(key) = contracts.targets().fuzzed_metric_key(&tx) {
                        metrics.entry(key).or_default().record_call(!result.status, rejected);
                    }
                    if rejected {
                        rejects += 1;
                        if rejects > options.max_assume_rejects {
                            for (test, failure) in tests.iter_mut().zip(&failures) {
                                if test.status.is_success() && failure.is_none() {
                                    test.status = TestStatus::Failure;
                                    test.reason = Some("Too many rejects (vm.assume)".to_owned());
                                }
                            }
                            break 'campaign;
                        }
                        continue;
                    }
                    dictionary.collect_call_output(
                        &contracts,
                        &tx,
                        &result.output,
                        &result.logs,
                        depth,
                    );
                    inspector = observed;
                    accepted += 1;
                    sequence.push(tx);
                    if !result.status {
                        reverts += 1;
                    }
                    if failed_assertion || (!result.status && options.fail_on_revert) {
                        let call = sequence.last().expect("failed call");
                        let site = FailureSite::Handler(
                            call.call_details.target,
                            inspector.reverter.unwrap_or(call.call_details.target),
                            alloy_primitives::Selector::from_slice(
                                &call.call_details.calldata[..4],
                            ),
                        );
                        let test =
                            self.outcome(result, failed_assertion, 0, inspector, timer.elapsed());
                        for (current, failure) in tests.iter_mut().zip(&mut failures) {
                            if current.status.is_success() && failure.is_none() {
                                *current = test.clone();
                                *failure = Some(Failure {
                                    site,
                                    result: test.clone(),
                                    calls: sequence.clone(),
                                    assertion: failed_assertion,
                                });
                            }
                        }
                        break 'campaign;
                    }
                    self.discover_invariant_contracts(&state, &targets, &contracts)?;
                    if (options.check_interval > 0
                        && accepted.is_multiple_of(options.check_interval))
                        || accepted == depth
                    {
                        for ((function, test), failure) in
                            functions.iter().zip(&mut tests).zip(&mut failures)
                        {
                            if test.status.is_success() && failure.is_none() {
                                *test = self.check_invariant(
                                    &state, &inspector, address, function, config, abi,
                                )?;
                                HitMaps::merge_opt(
                                    &mut campaign_coverage,
                                    test.line_coverage.take(),
                                );
                                if test.status.is_failure() {
                                    *failure = Some(Failure {
                                        site: FailureSite::Predicate(function.selector()),
                                        result: test.clone(),
                                        calls: sequence.clone(),
                                        assertion: false,
                                    });
                                }
                            }
                        }
                        if tests
                            .iter()
                            .zip(&failures)
                            .all(|(test, failure)| !test.status.is_success() || failure.is_some())
                        {
                            break 'campaign;
                        }
                    }
                }
                if abi.functions.contains_key("afterInvariant") {
                    let hook = abi.functions["afterInvariant"].first().expect("hook ABI");
                    let mut result =
                        self.check_invariant(&state, &inspector, address, hook, config, abi)?;
                    HitMaps::merge_opt(&mut campaign_coverage, result.line_coverage.take());
                    if !result.status.is_success() {
                        for (test, failure) in tests.iter_mut().zip(&mut failures) {
                            if test.status.is_success() && failure.is_none() {
                                *test = result.clone();
                                *failure = Some(Failure {
                                    site: FailureSite::AfterInvariant,
                                    result: result.clone(),
                                    calls: sequence.clone(),
                                    assertion: false,
                                });
                            }
                        }
                        break;
                    }
                }
            }
        }
        for (((function, test), failure), persisted) in
            functions.iter().zip(&mut tests).zip(failures).zip(persisted)
        {
            let replayed_failure = persisted;
            let failure_file =
                failure_dir.as_ref().map(|dir| dir.join("invariants").join(&function.name));
            if let Some(failure) = failure {
                let original_len = failure.calls.len();
                let failure = self.minimize_invariant(
                    executor, cheats, address, function, config, abi, &targets, failure,
                )?;
                let sequence = failure
                    .calls
                    .iter()
                    .map(|tx| {
                        BaseCounterExample::from_invariant_call(
                            tx,
                            &targets.setup,
                            None,
                            options.show_solidity,
                        )
                    })
                    .collect::<Vec<_>>();
                *test = failure.result;
                test.counterexample =
                    Some(CounterExample::Sequence(original_len, sequence.clone()));
                if functions.len() > 1
                    && failure.assertion
                    && let FailureSite::Handler(_, reverter, selector) = failure.site
                {
                    test.invariant_handler_failures = vec![InvariantFailure::Handler {
                        name: format!("{reverter}::{selector}"),
                        reverter,
                        selector,
                        reason: test.reason.clone().unwrap_or_default(),
                        counterexample: test.counterexample.clone(),
                        artifact: None,
                    }];
                }
                if let Some(path) = &failure_file {
                    let record = InvariantPersistedFailure {
                        call_sequence: sequence,
                        settings: settings.clone(),
                        assertion_failure: failure.assertion,
                        execution_failure: Some(InvariantFailureIdentity {
                            site: failure.site,
                            reason: test.reason.clone(),
                        }),
                        storage: Vec::new(),
                        failure_site: None,
                        fingerprint_provenance: None,
                    };
                    if let Some(parent) = path.parent()
                        && let Err(error) = foundry_common::fs::create_dir_all(parent)
                            .and_then(|()| foundry_common::fs::write_json_file(path, &record))
                    {
                        tracing::error!(%error, "Failed to record invariant counterexample");
                    }
                }
            }
            test.kind = TestKind::Invariant {
                runs: completed,
                calls,
                reverts,
                workers: 1,
                metrics: metrics.clone(),
                failed_corpus_replays: usize::from(replayed_failure),
                optimization_best_value: None,
            };
            test.merge_coverages(campaign_coverage.clone());
            test.duration = timer.elapsed();
        }
        Ok(tests)
    }

    fn invariant_call(
        &self,
        executor: &mut Executor<Database>,
        cheats: &Cheatcodes,
        tx: &mut BasicTxDetails,
        dictionary: Option<&FuzzState>,
    ) -> Result<(TxResult<FoundryEvmTypes>, Cheatcodes)> {
        let mut block = *executor.block();
        block.timestamp += tx.warp.unwrap_or_default();
        block.number += tx.roll.unwrap_or_default();
        executor.set_block(block);
        let balance = executor.account_info(tx.sender)?.map_or(U256::ZERO, |info| info.balance);
        let value = tx.call_details.value.unwrap_or_default().min(balance);
        tx.call_details.value = (!value.is_zero()).then_some(value);
        let mut request = self.tx(
            executor,
            tx.sender,
            TxKind::Call(tx.call_details.target),
            tx.call_details.calldata.clone(),
        )?;
        let TxEnvelope::Legacy(inner) = request.inner_mut() else {
            unreachable!("native synthetic request")
        };
        inner.value = value;
        let (result, inspector) = if let Some(dictionary) = dictionary {
            executor.inspect_transact_with_sink(
                &request,
                cheats.clone(),
                &mut DictionaryChanges(dictionary),
            )
        } else {
            executor.inspect_transact(&request, cheats.clone())
        };
        Ok((result?, inspector))
    }

    fn check_invariant(
        &self,
        executor: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
        function: &Function,
        config: &Config,
        abi: &JsonAbi,
    ) -> Result<TestResult> {
        let mut case = executor.clone();
        let tx = self.tx(
            executor,
            self.opts.sender,
            TxKind::Call(address),
            function.selector().to_vec().into(),
        )?;
        let (result, observed) = case.inspect_transact(&tx, cheats.clone());
        let result = result?;
        let assertion_failed = case.storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)? != U256::ZERO;
        let legacy_failed = result.status
            && config.legacy_assertions
            && abi.functions.contains_key("failed")
            && self.legacy_failed(&case, &observed, address)?;
        let mut test = self.outcome(result, assertion_failed, 0, observed, Default::default());
        if test.status.is_success() && legacy_failed {
            test.status = TestStatus::Failure;
            test.reason = Some("assertion failed".to_owned());
        }
        Ok(test)
    }
}

/// Samples borrowed finalized changes while the executor owns and commits the transaction.
struct DictionaryChanges<'a>(&'a FuzzState);

impl StateChangeSink for DictionaryChanges<'_> {
    type Error = Infallible;

    fn bytecode(&mut self, hash: B256, code: &Bytecode) -> Result<(), Self::Error> {
        self.0.collect_bytecode(hash, code.original_byte_slice());
        Ok(())
    }

    fn account(&mut self, change: AccountChangeRef<'_>) -> Result<(), Self::Error> {
        self.account_read(change.address, change.current)
    }

    fn account_read(
        &mut self,
        address: Address,
        info: Option<&AccountInfo>,
    ) -> Result<(), Self::Error> {
        self.0.collect_account(
            address,
            info.and_then(|info| {
                info.code.as_ref().map(|code| (info.code_hash, code.original_byte_slice()))
            }),
        );
        Ok(())
    }

    fn storage(&mut self, change: StorageChange) -> Result<(), Self::Error> {
        self.storage_read(change.address, change.key, change.current)
    }

    fn storage_read(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), Self::Error> {
        self.0.collect_storage(address, key, value);
        Ok(())
    }
}
