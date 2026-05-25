use crate::{
    executors::{
        DURATION_BETWEEN_METRICS_REPORT, EarlyExit, EvmError, Executor, FuzzTestTimer,
        RawCallResult, corpus::WorkerCorpus,
    },
    inspectors::Fuzzer,
};
use alloy_primitives::{Address, Bytes, FixedBytes, Selector, U256, map::AddressMap};
use alloy_sol_types::{SolCall, sol};
use eyre::{ContextCompat, Result, eyre};
use foundry_common::{
    TestFunctionExt,
    contracts::{ContractsByAddress, ContractsByArtifact},
    sh_println,
};
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    FoundryBlock,
    constants::{
        CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME,
    },
    evm::FoundryEvmNetwork,
    precompiles::PRECOMPILES,
};
use foundry_evm_fuzz::{
    BasicTxDetails, FuzzCase, FuzzFixtures,
    invariant::{
        ArtifactFilters, FuzzRunIdentifiedContracts, InvariantContract, InvariantSettings,
        RandomCallGenerator, SenderFilters, TargetedContract, TargetedContracts,
    },
    strategies::{EvmFuzzState, invariant_strat, override_call_strat},
};
use indicatif::ProgressBar;
use parking_lot::RwLock;
use proptest::{strategy::Strategy, test_runner::TestRunner};
use result::{assert_after_invariant, can_continue, did_fail_on_assert, invariant_preflight_check};
use revm::{context::Block, state::Account};
use serde::{Deserialize, Serialize};
use std::{
    collections::btree_map::Entry,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod campaign;
use campaign::{
    InvariantCampaign, InvariantFailureMetrics, InvariantThroughputMetrics, InvariantWorkerRun,
    build_invariant_progress_json, record_new_invariant_failures,
};

mod error;
pub use error::{
    FailureKey, HandlerAssertionFailure, InvariantFailures, InvariantFuzzError,
    handler_site_already_minimal,
};

mod replay;
pub use replay::{replay_error, replay_run};

mod result;
pub use campaign::InvariantFuzzTestResult;

mod shrink;
pub use shrink::{
    CheckSequenceOptions, HandlerReplayOutcome, check_sequence, check_sequence_value,
    replay_handler_failure_sequence,
};

sol! {
    interface IInvariantTest {
        #[derive(Default)]
        struct FuzzSelector {
            address addr;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzArtifactSelector {
            string artifact;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzInterface {
            address addr;
            string[] artifacts;
        }

        function afterInvariant() external;

        #[derive(Default)]
        function excludeArtifacts() public view returns (string[] memory excludedArtifacts);

        #[derive(Default)]
        function excludeContracts() public view returns (address[] memory excludedContracts);

        #[derive(Default)]
        function excludeSelectors() public view returns (FuzzSelector[] memory excludedSelectors);

        #[derive(Default)]
        function excludeSenders() public view returns (address[] memory excludedSenders);

        #[derive(Default)]
        function targetArtifacts() public view returns (string[] memory targetedArtifacts);

        #[derive(Default)]
        function targetArtifactSelectors() public view returns (FuzzArtifactSelector[] memory targetedArtifactSelectors);

        #[derive(Default)]
        function targetContracts() public view returns (address[] memory targetedContracts);

        #[derive(Default)]
        function targetSelectors() public view returns (FuzzSelector[] memory targetedSelectors);

        #[derive(Default)]
        function targetSenders() public view returns (address[] memory targetedSenders);

        #[derive(Default)]
        function targetInterfaces() public view returns (FuzzInterface[] memory targetedInterfaces);
    }
}

/// Contains invariant metrics for a single fuzzed selector.
#[derive(Default, Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct InvariantMetrics {
    // Count of fuzzed selector calls.
    pub calls: usize,
    // Count of fuzzed selector reverts.
    pub reverts: usize,
    // Count of fuzzed selector discards (through assume cheatcodes).
    pub discards: usize,
}

/// Wrapper around any [`Executor`] implementer which provides fuzzing support using [`proptest`].
///
/// After instantiation, calling `invariant_fuzz` will proceed to hammer the deployed smart
/// contracts with inputs, until it finds a counterexample sequence. The provided [`TestRunner`]
/// contains all the configuration which can be overridden via [environment
/// variables](proptest::test_runner::Config)
pub struct InvariantExecutor<'a, FEN: FoundryEvmNetwork> {
    pub executor: Executor<FEN>,
    /// Proptest runner.
    runner: TestRunner,
    /// The invariant configuration
    config: InvariantConfig,
    /// Contracts deployed with `setUp()`
    setup_contracts: &'a ContractsByAddress,
    /// Contracts that are part of the project but have not been deployed yet. We need the bytecode
    /// to identify them from the stateset changes.
    project_contracts: &'a ContractsByArtifact,
    /// Filters contracts to be fuzzed through their artifact identifiers.
    artifact_filters: ArtifactFilters,
}

impl<'a, FEN: FoundryEvmNetwork> InvariantExecutor<'a, FEN> {
    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(
        executor: Executor<FEN>,
        runner: TestRunner,
        config: InvariantConfig,
        setup_contracts: &'a ContractsByAddress,
        project_contracts: &'a ContractsByArtifact,
    ) -> Self {
        Self {
            executor,
            runner,
            config,
            setup_contracts,
            project_contracts,
            artifact_filters: ArtifactFilters::default(),
        }
    }

    pub fn config(self) -> InvariantConfig {
        self.config
    }

    /// Fuzzes any deployed contract and checks any broken invariant at `invariant_address`.
    ///
    /// `initial_handler_failures` pre-seeds the campaign's `broken_handlers` map with bugs
    /// recovered from disk by the runner's persisted-failure replay step, so the live
    /// progress bar and JSON pulse stream surface them from the first emission instead of
    /// jumping at the final report.
    pub fn invariant_fuzz(
        &mut self,
        invariant_contract: InvariantContract<'_>,
        fuzz_fixtures: &FuzzFixtures,
        fuzz_state: EvmFuzzState,
        progress: Option<&ProgressBar>,
        early_exit: &EarlyExit,
        initial_handler_failures: std::collections::HashMap<
            (Address, Selector),
            InvariantFuzzError,
        >,
    ) -> Result<InvariantFuzzTestResult> {
        // Note: invariant function signatures (no inputs) are validated upstream in the
        // suite runner so parameterized `invariant_*` functions are rejected with a per-test
        // failure entry before any campaign runs.

        let (mut campaign, mut corpus_manager) = self.prepare_test(
            &invariant_contract,
            fuzz_fixtures,
            fuzz_state,
            initial_handler_failures,
        )?;

        // Start timer for this invariant test.
        let mut runs = 0;
        let timer = FuzzTestTimer::new(self.config.timeout);
        let mut last_metrics_report = Instant::now();
        let campaign_start = Instant::now();
        let mut throughput = InvariantThroughputMetrics::default();
        let mut failure_metrics = InvariantFailureMetrics::default();
        let continue_campaign = |runs: u32| {
            if early_exit.should_stop() {
                return false;
            }

            if timer.is_enabled() { !timer.is_timed_out() } else { runs < self.config.runs }
        };

        // Invariant runs with edge coverage if corpus dir is set or showing edge coverage.
        let edge_coverage_enabled = self.config.corpus.collect_edge_coverage();

        'stop: while continue_campaign(runs) {
            // Per-run failure count snapshot used to gate `afterInvariant` below.
            let failures_before_run = campaign.state.failures.invariant_count();
            let mut stop_after_run = false;

            let initial_seq = corpus_manager.new_inputs(
                &mut campaign.state.branch_runner,
                &campaign.fuzz_state,
                &campaign.targeted_contracts,
            )?;

            // Create current invariant run data.
            let mut current_run = InvariantWorkerRun::new(
                initial_seq[0].clone(),
                // Before each run, we must reset the backend state.
                self.executor.clone(),
                self.config.depth as usize,
            );

            // We stop the run immediately if we have reverted, and `fail_on_revert` is set.
            if self.config.fail_on_revert && campaign.reverts() > 0 {
                return Err(eyre!("call reverted"));
            }

            while current_run.depth < self.config.depth {
                // Check if the timeout has been reached.
                if timer.is_timed_out() {
                    // Since we never record a revert here the test is still considered
                    // successful even though it timed out. We *want*
                    // this behavior for now, so that's ok, but
                    // future developers should be aware of this.
                    break 'stop;
                }

                // Snapshot `(target, selector)` so `can_continue` can borrow `&mut current_run`
                // later without cloning the full `BasicTxDetails`.
                let (handler_target, handler_selector) = {
                    let last = current_run
                        .inputs
                        .last()
                        .ok_or_else(|| eyre!("no input generated to call fuzzed target."))?;
                    let sel_bytes: [u8; 4] = last
                        .call_details
                        .calldata
                        .get(..4)
                        .and_then(|s| s.try_into().ok())
                        .unwrap_or_default();
                    (last.call_details.target, Selector::from(sel_bytes))
                };

                // Execute call from the randomly generated sequence without committing state.
                // State is committed only if call is not a magic assume.
                let mut call_result = execute_tx(
                    &mut current_run.executor,
                    current_run.inputs.last().expect("checked above"),
                )?;
                if let Some(fuzzer) = current_run.executor.inspector_mut().fuzzer.as_mut() {
                    campaign.fuzz_state.collect_values(fuzzer.drain_collected_values());
                }
                // Capture per-call EVM cmp operands for I2S corpus mutation. Kept parallel
                // to `current_run.inputs`; populated unconditionally so dropped calls (magic
                // assumes / pops below) get zero-length entries that the corpus side filters out.
                let call_cmp_values = call_result.evm_cmp_values.take().unwrap_or_default();
                let discarded = call_result.result.as_ref() == MAGIC_ASSUME;
                if self.config.show_metrics {
                    campaign.record_metrics(
                        current_run.inputs.last().expect("checked above"),
                        call_result.reverted,
                        discarded,
                    );
                }

                // Collect line coverage from last fuzzed call.
                campaign.merge_line_coverage(call_result.line_coverage.clone());
                // Snapshot the edge fingerprint before `merge_edge_coverage` zeroes the
                // buffer. Gate on `assertion_failure` to skip keccak on plain reverts.
                let assertion_failure =
                    !discarded && did_fail_on_assert(&call_result, &call_result.state_changeset);
                let pre_merge_edges_hash = if assertion_failure {
                    error::snapshot_edge_fingerprint(&call_result)
                } else {
                    None
                };
                // Collect edge coverage and set the flag in the current run.
                if corpus_manager.merge_edge_coverage(&mut call_result) {
                    current_run.new_coverage = true;
                }

                if discarded {
                    current_run.inputs.pop();
                    current_run.rejects += 1;
                    if current_run.rejects > self.config.max_assume_rejects {
                        campaign.set_error(
                            invariant_contract.anchor(),
                            InvariantFuzzError::MaxAssumeRejects(self.config.max_assume_rejects),
                        );
                        break 'stop;
                    }
                } else {
                    // Commit executed call result.
                    current_run.executor.commit(&mut call_result);

                    // Collect data for fuzzing from the state changeset.
                    // This step updates the state dictionary and therefore invalidates the
                    // ValueTree in use by the current run. This manifestsitself in proptest
                    // observing a different input case than what it was called with, and creates
                    // inconsistencies whenever proptest tries to use the input case after test
                    // execution.
                    // See <https://github.com/foundry-rs/foundry/issues/9764>.
                    let mut state_changeset = std::mem::take(&mut call_result.state_changeset);
                    if !call_result.reverted {
                        let mapping_slots = current_run
                            .executor
                            .inspector()
                            .fuzzer
                            .as_ref()
                            .and_then(|fuzzer| fuzzer.mapping_slots.as_ref());
                        collect_data(
                            &campaign,
                            &mut state_changeset,
                            current_run.inputs.last().expect("checked above"),
                            &call_result,
                            self.config.depth,
                            mapping_slots,
                        );
                    }

                    // Collect created contracts and add to fuzz targets only if targeted contracts
                    // are updatable.
                    if let Err(error) = &campaign.targeted_contracts.collect_created_contracts(
                        &state_changeset,
                        self.project_contracts,
                        self.setup_contracts,
                        &self.artifact_filters,
                        &mut current_run.created_contracts,
                    ) {
                        warn!(target: "forge::test", "{error}");
                    }
                    current_run
                        .fuzz_runs
                        .push(FuzzCase { gas: call_result.gas_used, stipend: call_result.stipend });
                    throughput.record_call(call_result.gas_used);

                    // Determine if test can continue or should exit.
                    // Check invariants based on check_interval to improve deep run performance.
                    // - check_interval=0: only assert on the last call
                    // - check_interval=1 (default): assert after every call
                    // - check_interval=N: assert every N calls AND always on the last call
                    let is_last_call = current_run.depth == self.config.depth - 1;
                    // In optimization mode, always evaluate the invariant to track
                    // the best value at every prefix — check_interval only gates
                    // boolean invariant assertions.
                    let is_optimization = invariant_contract.is_optimization();
                    let should_check_invariant = is_optimization
                        || if self.config.check_interval == 0 {
                            is_last_call
                        } else {
                            self.config.check_interval == 1
                                || (current_run.depth + 1)
                                    .is_multiple_of(self.config.check_interval)
                                || is_last_call
                        };

                    let errors_before_check = campaign.state.failures.invariant_count();
                    let (continues, broken) = if should_check_invariant {
                        let outcome = can_continue(
                            &invariant_contract,
                            &mut campaign,
                            &mut current_run,
                            &self.config,
                            call_result,
                            &state_changeset,
                            handler_target,
                            handler_selector,
                            pre_merge_edges_hash,
                        )
                        .map_err(|e| eyre!(e.to_string()))?;
                        (outcome.continues, outcome.broken)
                    } else {
                        // Skip invariant check but still track reverts
                        if call_result.reverted {
                            campaign.state.failures.reverts += 1;
                        }
                        if assertion_failure {
                            // Handler-side assertion: deduped by `(reverter, selector)` site;
                            // campaign keeps running to surface more bugs.
                            let call_reverted = call_result.reverted;
                            error::record_handler_assertion_bug(
                                &invariant_contract,
                                &self.config,
                                &campaign.targeted_contracts,
                                &mut campaign.state.failures,
                                &mut current_run.inputs,
                                handler_target,
                                handler_selector,
                                pre_merge_edges_hash,
                                call_result,
                                call_reverted,
                                invariant_contract.is_optimization(),
                            );
                            (true, None)
                        } else if call_result.reverted && self.config.fail_on_revert {
                            // Plain revert under fail_on_revert: attribute to the anchor.
                            let anchor = invariant_contract.anchor();
                            let case_data = error::InvariantRunCtx {
                                contract: &invariant_contract,
                                config: &self.config,
                                targeted_contracts: &campaign.targeted_contracts,
                                calldata: &current_run.inputs,
                            }
                            .failed_case(
                                anchor,
                                self.config.fail_on_revert,
                                false,
                                call_result,
                                &[],
                            );
                            campaign.set_error(anchor, InvariantFuzzError::Revert(case_data));
                            (false, Some(anchor))
                        } else if call_result.reverted
                            && !invariant_contract.is_optimization()
                            && !self.config.has_delay()
                        {
                            // Delay campaigns keep reverted calls so warp/roll survives shrinking.
                            current_run.inputs.pop();
                            (true, None)
                        } else {
                            (true, None)
                        }
                    };

                    // Keep `cmp_seq` parallel to `inputs`: only push when the input survived the
                    // pop branch above.
                    if current_run.cmp_seq.len() < current_run.inputs.len() {
                        current_run.cmp_seq.push(call_cmp_values);
                    }

                    if !continues || current_run.depth == self.config.depth - 1 {
                        campaign.set_last_run_inputs(&current_run.inputs);
                    }
                    // Bridge newly-recorded predicate breaks into `failure_metrics` even when
                    // `continues == true` in multi-predicate campaigns.
                    if campaign.state.failures.invariant_count() > errors_before_check
                        || broken.is_some()
                    {
                        record_new_invariant_failures(
                            &mut failure_metrics,
                            &invariant_contract,
                            &campaign.state.failures,
                        );
                    }
                    if !continues {
                        if invariant_contract.invariant_fns.len() > 1 && !self.config.fail_on_revert
                        {
                            break;
                        }
                        stop_after_run = true;
                        break;
                    }
                    current_run.depth += 1;
                }

                current_run.inputs.push(corpus_manager.generate_next_input(
                    &mut campaign.state.branch_runner,
                    &initial_seq,
                    discarded,
                    current_run.depth as usize,
                )?);
            }

            // Extend corpus with current run data.
            // Materialize the optimization best prefix once at run end (avoids
            // cloning inputs on every new in-run max).
            let optimization = current_run.optimization_value.map(|v| {
                let prefix = current_run.inputs[..current_run.optimization_prefix_len].to_vec();
                (v, prefix)
            });
            corpus_manager.process_inputs(
                &current_run.inputs,
                &current_run.cmp_seq,
                current_run.new_coverage,
                optimization,
            );

            // Call `afterInvariant` only if declared and the current run produced no new
            // failure. Multi-predicate campaigns keep running after earlier failures, but the
            // hook must still execute on subsequent runs.
            if invariant_contract.call_after_invariant
                && campaign.state.failures.invariant_count() == failures_before_run
            {
                let broken = assert_after_invariant(
                    &invariant_contract,
                    &mut campaign,
                    &current_run,
                    &self.config,
                )
                .map_err(|_| eyre!("Failed to call afterInvariant"))?;
                if broken.is_some() {
                    // Bridge breaks into pulse metrics, mirroring the in-run path above.
                    record_new_invariant_failures(
                        &mut failure_metrics,
                        &invariant_contract,
                        &campaign.state.failures,
                    );
                }
            }

            // End current invariant test run.
            campaign.end_run(current_run, self.config.gas_report_samples as usize);
            if let Some(progress) = progress {
                // If running with progress then increment completed runs.
                progress.inc(1);
                // Display current best value, corpus metrics, and failure counts.
                let best = campaign.state.optimization_best_value;
                let broken = campaign.state.failures.invariant_count();
                // Live count of unique handler-side assertion bugs, separate from the
                // predicate breaks in `broken`. Synced into `failure_metrics` so all
                // campaign-level counters share one struct.
                failure_metrics.broken_handlers = campaign.state.failures.handler_count();
                let handler_bugs = failure_metrics.broken_handlers;
                let total_invariants = invariant_contract.invariant_fns.len();
                if edge_coverage_enabled || best.is_some() || broken > 0 || handler_bugs > 0 {
                    let mut msg = String::new();
                    if let Some(best) = best {
                        msg.push_str(&format!("best: {best}"));
                    }
                    if edge_coverage_enabled {
                        if !msg.is_empty() {
                            msg.push_str(", ");
                        }
                        msg.push_str(&format!("{}", corpus_manager.metrics));
                    }
                    if broken > 0 {
                        if !msg.is_empty() {
                            msg.push_str(", ");
                        }
                        msg.push_str(&format!("❌ {broken}/{total_invariants} broken"));
                    }
                    if handler_bugs > 0 {
                        if !msg.is_empty() {
                            msg.push_str(", ");
                        }
                        msg.push_str(&format!("⚠ {handler_bugs} handler bug(s)"));
                    }
                    progress.set_message(msg);
                }
            } else if edge_coverage_enabled
                && last_metrics_report.elapsed() > DURATION_BETWEEN_METRICS_REPORT
            {
                // Sync handler-bug count snapshot into failure_metrics before emitting.
                failure_metrics.broken_handlers = campaign.state.failures.handler_count();
                // Display corpus metrics inline as JSON.
                let metrics = build_invariant_progress_json(
                    SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    &invariant_contract.anchor().name,
                    &corpus_manager.metrics,
                    campaign.state.optimization_best_value,
                    throughput,
                    &failure_metrics,
                    campaign_start.elapsed(),
                );
                let _ = sh_println!("{}", serde_json::to_string(&metrics)?);
                last_metrics_report = Instant::now();
            }

            runs += 1;
            if stop_after_run {
                break 'stop;
            }
        }

        trace!(?fuzz_fixtures);
        campaign.fuzz_state.log_stats();

        let mut result = campaign.state;

        // Post-campaign: shrink each handler bug's call sequence to its minimal prefix.
        let total = result.failures.handler_count();
        if total > 0 {
            for (idx, (_site, error)) in result.failures.handler_failures_mut().enumerate() {
                if early_exit.should_stop() {
                    break;
                }
                let Some(failure) = error.as_handler_assertion_mut() else {
                    // Handler-keyed entries always store `HandlerAssertion` by construction.
                    continue;
                };
                shrink::reset_shrink_progress(
                    &self.config,
                    progress,
                    &format!("handler {:#x}::{}", failure.reverter, failure.selector),
                    Some((idx + 1, total)),
                );
                match shrink::shrink_handler_sequence(
                    &self.config,
                    &failure.call_sequence,
                    failure.edge_fingerprint,
                    &self.executor,
                    progress,
                    early_exit,
                ) {
                    Ok(shrunk) if !shrunk.is_empty() => {
                        failure.call_sequence = shrunk;
                    }
                    Ok(_) => {}
                    Err(e) => trace!(target: "forge::test", "handler shrink failed: {e}"),
                }
            }
        }

        Ok(result.into_fuzz_result(corpus_manager.failed_replays))
    }

    /// Prepares certain structures to execute the invariant tests:
    /// * Invariant Fuzz Test.
    /// * Invariant Corpus Manager.
    fn prepare_test(
        &mut self,
        invariant_contract: &InvariantContract<'_>,
        fuzz_fixtures: &FuzzFixtures,
        fuzz_state: EvmFuzzState,
        initial_handler_failures: std::collections::HashMap<
            (Address, Selector),
            InvariantFuzzError,
        >,
    ) -> Result<(InvariantCampaign, WorkerCorpus)> {
        // Finds out the chosen deployed contracts and/or senders.
        self.select_contract_artifacts(invariant_contract.address)?;
        let (targeted_senders, targeted_contracts) =
            self.select_contracts_and_senders(invariant_contract.address)?;
        let fuzz_state = fuzz_state.into_invariant();

        // Creates the invariant strategy.
        let strategy = invariant_strat(
            fuzz_state.clone(),
            targeted_senders,
            targeted_contracts.clone(),
            self.config.clone(),
            fuzz_fixtures.clone(),
        )
        .no_shrink();

        // If any of the targeted contracts have the storage layout enabled then we can sample
        // mapping values. To accomplish, we need to record the mapping storage slots and keys.
        let mapping_slots = targeted_contracts
            .targets()
            .iter()
            .any(|(_, t)| t.storage_layout.is_some())
            .then(AddressMap::default);

        // Set up fuzzer WITHOUT call_generator initially.
        // We defer call_override until after the initial invariant check to avoid
        // injecting random calls during setup which would break the invariant assertion.
        self.executor.inspector_mut().set_fuzzer(Fuzzer {
            call_generator: None,
            collected_values: Vec::new(),
            max_collected_values: self.config.dictionary.max_fuzz_dictionary_values,
            mapping_slots,
            collect: true,
        });

        // Let's make sure the invariant is sound before actually starting the run:
        // We'll assert the invariant in its initial state, and if it fails, we'll
        // already know if we can early exit the invariant run.
        // This does not count as a fuzz run. It will just register the revert.
        let mut failures = InvariantFailures::new();
        // Seed disk-recovered handler bugs so live counters reflect them from tick 0.
        for ((addr, sel), err) in initial_handler_failures {
            failures.seed_handler_failure(addr, sel, err);
        }
        invariant_preflight_check(
            invariant_contract,
            &self.config,
            &targeted_contracts,
            &self.executor,
            &[],
            &mut failures,
        )?;
        if let Some(fuzzer) = self.executor.inspector_mut().fuzzer.as_mut() {
            fuzz_state.collect_values(fuzzer.drain_collected_values());
        }
        // NOW enable call_override after the initial invariant check has passed.
        // This allows `override_call_strat` to inject calls during actual fuzz runs
        // for reentrancy vulnerability detection.
        if self.config.call_override {
            let target_contract_ref = Arc::new(RwLock::new(Address::ZERO));

            // Collect handler addresses - these are the contracts we want to inject
            // reentrancy into (simulating malicious receive() functions).
            let handler_addresses: std::collections::HashSet<Address> =
                targeted_contracts.targets().keys().copied().collect();
            let override_targets = targeted_contracts
                .targets()
                .iter()
                .filter_map(|(address, contract)| {
                    let functions = contract.abi_fuzzed_functions().cloned().collect::<Vec<_>>();
                    (!functions.is_empty()).then_some((*address, functions))
                })
                .collect::<Vec<_>>();

            let call_generator = RandomCallGenerator::new(
                invariant_contract.address,
                handler_addresses,
                self.runner.clone(),
                override_call_strat(
                    fuzz_state.snapshot(),
                    override_targets,
                    target_contract_ref.clone(),
                    fuzz_fixtures.clone(),
                ),
                target_contract_ref,
            );

            if let Some(fuzzer) = self.executor.inspector_mut().fuzzer.as_mut() {
                fuzzer.call_generator = Some(call_generator);
            }
        }

        let worker = WorkerCorpus::new(
            0,
            self.config.corpus.clone(),
            strategy.boxed(),
            Some(&self.executor),
            None,
            Some(&targeted_contracts),
        )?;

        let mut campaign =
            InvariantCampaign::new(fuzz_state, targeted_contracts, failures, self.runner.clone());

        // Seed invariant test with previously persisted optimization state,
        // but only if the current invariant is in optimization mode.
        if invariant_contract.is_optimization() {
            let (opt_best_value, opt_best_sequence) = worker.optimization_initial_state();
            campaign.state.optimization_best_value = opt_best_value;
            campaign.state.optimization_best_sequence = opt_best_sequence;
        }

        Ok((campaign, worker))
    }

    /// Fills the `InvariantExecutor` with the artifact identifier filters (in `path:name` string
    /// format). They will be used to filter contracts after the `setUp`, and more importantly,
    /// during the runs.
    ///
    /// Also excludes any contract without any mutable functions.
    ///
    /// Priority:
    ///
    /// targetArtifactSelectors > excludeArtifacts > targetArtifacts
    pub fn select_contract_artifacts(&mut self, invariant_address: Address) -> Result<()> {
        let targeted_artifact_selectors = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::targetArtifactSelectorsCall {});

        // Insert them into the executor `targeted_abi`.
        for IInvariantTest::FuzzArtifactSelector { artifact, selectors } in
            targeted_artifact_selectors
        {
            let identifier = self.validate_selected_contract(artifact, &selectors)?;
            self.artifact_filters.targeted.entry(identifier).or_default().extend(selectors);
        }

        let targeted_artifacts = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::targetArtifactsCall {});
        let excluded_artifacts = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::excludeArtifactsCall {});

        // Insert `excludeArtifacts` into the executor `excluded_abi`.
        for contract in excluded_artifacts {
            let identifier = self.validate_selected_contract(contract, &[])?;

            if !self.artifact_filters.excluded.contains(&identifier) {
                self.artifact_filters.excluded.push(identifier);
            }
        }

        // Exclude any artifact without mutable functions.
        for (artifact, contract) in self.project_contracts.iter() {
            if contract
                .abi
                .functions()
                .filter(|func| {
                    !matches!(
                        func.state_mutability,
                        alloy_json_abi::StateMutability::Pure
                            | alloy_json_abi::StateMutability::View
                    )
                })
                .count()
                == 0
                && !self.artifact_filters.excluded.contains(&artifact.identifier())
            {
                self.artifact_filters.excluded.push(artifact.identifier());
            }
        }

        // Insert `targetArtifacts` into the executor `targeted_abi`, if they have not been seen
        // before.
        for contract in targeted_artifacts {
            let identifier = self.validate_selected_contract(contract, &[])?;

            if !self.artifact_filters.targeted.contains_key(&identifier)
                && !self.artifact_filters.excluded.contains(&identifier)
            {
                self.artifact_filters.targeted.insert(identifier, vec![]);
            }
        }
        Ok(())
    }

    /// Makes sure that the contract exists in the project. If so, it returns its artifact
    /// identifier.
    fn validate_selected_contract(
        &mut self,
        contract: String,
        selectors: &[FixedBytes<4>],
    ) -> Result<String> {
        if let Some((artifact, contract_data)) =
            self.project_contracts.find_by_name_or_identifier(&contract)?
        {
            // Check that the selectors really exist for this contract.
            for selector in selectors {
                contract_data
                    .abi
                    .functions()
                    .find(|func| func.selector().as_slice() == selector.as_slice())
                    .wrap_err(format!("{contract} does not have the selector {selector:?}"))?;
            }

            return Ok(artifact.identifier());
        }
        eyre::bail!(
            "{contract} not found in the project. Allowed format: `contract_name` or `contract_path:contract_name`."
        );
    }

    /// Selects senders and contracts based on the contract methods `targetSenders() -> address[]`,
    /// `targetContracts() -> address[]` and `excludeContracts() -> address[]`.
    pub fn select_contracts_and_senders(
        &self,
        to: Address,
    ) -> Result<(SenderFilters, FuzzRunIdentifiedContracts)> {
        let targeted_senders =
            self.executor.call_sol_default(to, &IInvariantTest::targetSendersCall {});
        let mut excluded_senders =
            self.executor.call_sol_default(to, &IInvariantTest::excludeSendersCall {});
        // Extend with default excluded addresses - https://github.com/foundry-rs/foundry/issues/4163
        excluded_senders.extend([
            CHEATCODE_ADDRESS,
            HARDHAT_CONSOLE_ADDRESS,
            DEFAULT_CREATE2_DEPLOYER,
        ]);
        // Extend with precompiles - https://github.com/foundry-rs/foundry/issues/4287
        excluded_senders.extend(PRECOMPILES);
        let sender_filters = SenderFilters::new(targeted_senders, excluded_senders);

        let selected = self.executor.call_sol_default(to, &IInvariantTest::targetContractsCall {});
        let excluded = self.executor.call_sol_default(to, &IInvariantTest::excludeContractsCall {});

        let contracts = self
            .setup_contracts
            .iter()
            .filter(|&(addr, (identifier, _))| {
                // Include to address if explicitly set as target.
                if *addr == to && selected.contains(&to) {
                    return true;
                }

                *addr != to
                    && *addr != CHEATCODE_ADDRESS
                    && *addr != HARDHAT_CONSOLE_ADDRESS
                    && (selected.is_empty() || selected.contains(addr))
                    && (excluded.is_empty() || !excluded.contains(addr))
                    && self.artifact_filters.matches(identifier)
            })
            .map(|(addr, (identifier, abi))| {
                (
                    *addr,
                    TargetedContract::new(identifier.clone(), abi.clone())
                        .with_project_contracts(self.project_contracts),
                )
            })
            .collect();
        let mut contracts = TargetedContracts { inner: contracts };

        self.target_interfaces(to, &mut contracts)?;

        self.select_selectors(to, &mut contracts)?;

        // There should be at least one contract identified as target for fuzz runs.
        if contracts.is_empty() {
            eyre::bail!("No contracts to fuzz.");
        }

        Ok((sender_filters, FuzzRunIdentifiedContracts::new(contracts, selected.is_empty())))
    }

    /// Extends the contracts and selectors to fuzz with the addresses and ABIs specified in
    /// `targetInterfaces() -> (address, string[])[]`. Enables targeting of addresses that are
    /// not deployed during `setUp` such as when fuzzing in a forked environment. Also enables
    /// targeting of delegate proxies and contracts deployed with `create` or `create2`.
    pub fn target_interfaces(
        &self,
        invariant_address: Address,
        targeted_contracts: &mut TargetedContracts,
    ) -> Result<()> {
        let interfaces = self
            .executor
            .call_sol_default(invariant_address, &IInvariantTest::targetInterfacesCall {});

        // Since `targetInterfaces` returns a tuple array there is no guarantee
        // that the addresses are unique this map is used to merge functions of
        // the specified interfaces for the same address. For example:
        // `[(addr1, ["IERC20", "IOwnable"])]` and `[(addr1, ["IERC20"]), (addr1, ("IOwnable"))]`
        // should be equivalent.
        let mut combined = TargetedContracts::new();

        // Loop through each address and its associated artifact identifiers.
        // We're borrowing here to avoid taking full ownership.
        for IInvariantTest::FuzzInterface { addr, artifacts } in &interfaces {
            // Identifiers are specified as an array, so we loop through them.
            for identifier in artifacts {
                // Try to find the contract by name or identifier in the project's contracts.
                if let Some((_, contract_data)) =
                    self.project_contracts.iter().find(|(artifact, _)| {
                        &artifact.name == identifier || &artifact.identifier() == identifier
                    })
                {
                    let abi = &contract_data.abi;
                    combined
                        // Check if there's an entry for the given key in the 'combined' map.
                        .entry(*addr)
                        // If the entry exists, extends its ABI with the function list.
                        .and_modify(|entry| {
                            // Extend the ABI's function list with the new functions.
                            entry.abi.functions.extend(abi.functions.clone());
                        })
                        // Otherwise insert it into the map.
                        .or_insert_with(|| {
                            let mut contract =
                                TargetedContract::new(identifier.clone(), abi.clone());
                            contract.storage_layout =
                                contract_data.storage_layout.as_ref().map(Arc::clone);
                            contract
                        });
                }
            }
        }

        targeted_contracts.extend(combined.inner);

        Ok(())
    }

    /// Selects the functions to fuzz based on the contract method `targetSelectors()` and
    /// `targetArtifactSelectors()`.
    pub fn select_selectors(
        &self,
        address: Address,
        targeted_contracts: &mut TargetedContracts,
    ) -> Result<()> {
        for (address, (identifier, _)) in self.setup_contracts {
            if let Some(selectors) = self.artifact_filters.targeted.get(identifier) {
                self.add_address_with_functions(*address, selectors, false, targeted_contracts)?;
            }
        }

        let mut target_test_selectors = vec![];
        let mut excluded_test_selectors = vec![];

        // Collect contract functions marked as target for fuzzing campaign.
        let selectors =
            self.executor.call_sol_default(address, &IInvariantTest::targetSelectorsCall {});
        for IInvariantTest::FuzzSelector { addr, selectors } in selectors {
            if addr == address {
                target_test_selectors = selectors.clone();
            }
            self.add_address_with_functions(addr, &selectors, false, targeted_contracts)?;
        }

        // Collect contract functions excluded from fuzzing campaign.
        let excluded_selectors =
            self.executor.call_sol_default(address, &IInvariantTest::excludeSelectorsCall {});
        for IInvariantTest::FuzzSelector { addr, selectors } in excluded_selectors {
            if addr == address {
                // If fuzz selector address is the test contract, then record selectors to be
                // later excluded if needed.
                excluded_test_selectors = selectors.clone();
            }
            self.add_address_with_functions(addr, &selectors, true, targeted_contracts)?;
        }

        if target_test_selectors.is_empty()
            && let Some(target) = targeted_contracts.get(&address)
        {
            // If test contract is marked as a target and no target selector explicitly set, then
            // include only state-changing functions that are not reserved and selectors that are
            // not explicitly excluded.
            let selectors: Vec<_> = target
                .abi
                .functions()
                .filter_map(|func| {
                    if matches!(
                        func.state_mutability,
                        alloy_json_abi::StateMutability::Pure
                            | alloy_json_abi::StateMutability::View
                    ) || func.is_reserved()
                        || excluded_test_selectors.contains(&func.selector())
                    {
                        None
                    } else {
                        Some(func.selector())
                    }
                })
                .collect();
            self.add_address_with_functions(address, &selectors, false, targeted_contracts)?;
        }

        Ok(())
    }

    /// Adds the address and fuzzed or excluded functions to `TargetedContracts`.
    fn add_address_with_functions(
        &self,
        address: Address,
        selectors: &[Selector],
        should_exclude: bool,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        // Do not add address in target contracts if no function selected.
        if selectors.is_empty() {
            return Ok(());
        }

        let contract = match targeted_contracts.entry(address) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let (identifier, abi) = self.setup_contracts.get(&address).ok_or_else(|| {
                    eyre::eyre!(
                        "[{}] address does not have an associated contract: {}",
                        if should_exclude { "excludeSelectors" } else { "targetSelectors" },
                        address
                    )
                })?;
                entry.insert(
                    TargetedContract::new(identifier.clone(), abi.clone())
                        .with_project_contracts(self.project_contracts),
                )
            }
        };
        contract.add_selectors(selectors.iter().copied(), should_exclude)?;
        Ok(())
    }

    /// Computes the current invariant settings for the given invariant contract address.
    ///
    /// This extracts the target contracts, selectors, senders, and failure settings
    /// that are used to determine if a persisted counterexample is still valid.
    pub fn compute_settings(&mut self, invariant_address: Address) -> Result<InvariantSettings> {
        self.select_contract_artifacts(invariant_address)?;
        let (sender_filters, targeted_contracts) =
            self.select_contracts_and_senders(invariant_address)?;
        let targets = targeted_contracts.targets();
        Ok(InvariantSettings::new(&targets, &sender_filters, self.config.fail_on_revert))
    }
}

/// Collects data from call for fuzzing. However, it first verifies that the sender is not an EOA
/// before inserting it into the dictionary. Otherwise, we flood the dictionary with
/// randomly generated addresses.
fn collect_data<FEN: FoundryEvmNetwork>(
    campaign: &InvariantCampaign,
    state_changeset: &mut AddressMap<Account>,
    tx: &BasicTxDetails,
    call_result: &RawCallResult<FEN>,
    run_depth: u32,
    mapping_slots: Option<&AddressMap<foundry_common::mapping_slots::MappingSlots>>,
) {
    // Verify it has no code.
    let has_code = if let Some(Some(code)) =
        state_changeset.get(&tx.sender).map(|account| account.info.code.as_ref())
    {
        !code.is_empty()
    } else {
        false
    };

    // We keep the nonce changes to apply later.
    let sender_changeset = if has_code { None } else { state_changeset.remove(&tx.sender) };

    // Collect values from fuzzed call result and add them to fuzz dictionary.
    campaign.fuzz_state.collect_values_from_call(
        &campaign.targeted_contracts,
        tx,
        &call_result.result,
        &call_result.logs,
        &*state_changeset,
        run_depth,
        mapping_slots,
    );

    // Inject typed sancov trace-cmp operands into the fuzz dictionary.
    if let Some(cmp_values) = &call_result.sancov_cmp_values {
        campaign.fuzz_state.collect_typed_cmp_values(
            cmp_values.iter().map(|s| (s.width, alloy_primitives::B256::from(s.value))),
        );
    }
    // Re-add changes
    if let Some(changed) = sender_changeset {
        state_changeset.insert(tx.sender, changed);
    }
}

/// Calls the `afterInvariant()` function on a contract.
/// Returns call result and if call succeeded.
/// The state after the call is not persisted.
///
/// Uses the handler-gate success check so a stale committed `GLOBAL_FAIL_SLOT` from a
/// previously-recorded handler bug doesn't false-positive this call (the slot is `1` from
/// the prior bug, but `afterInvariant` itself didn't write it in this changeset).
pub(crate) fn call_after_invariant_function<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    to: Address,
) -> Result<(RawCallResult<FEN>, bool), EvmError<FEN>> {
    let calldata = Bytes::from_static(&IInvariantTest::afterInvariantCall::SELECTOR);
    let mut call_result = executor.call_raw(CALLER, to, calldata, U256::ZERO)?;
    let success = executor.is_raw_call_mut_success_handler_gate(to, &mut call_result);
    Ok((call_result, success))
}

/// Calls the invariant function and returns call result and if succeeded.
///
/// Uses the handler-gate success check (same rationale as `call_after_invariant_function`):
/// the predicate is broken iff this call's own changeset writes `GLOBAL_FAIL_SLOT` (via `t()` /
/// `vm.assert*`) or the call reverts; a stale committed slot from a prior handler bug must not
/// poison every later predicate evaluation in the run.
pub(crate) fn call_invariant_function<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    address: Address,
    calldata: Bytes,
) -> Result<(RawCallResult<FEN>, bool)> {
    let mut call_result = executor.call_raw(CALLER, address, calldata, U256::ZERO)?;
    let success = executor.is_raw_call_mut_success_handler_gate(address, &mut call_result);
    Ok((call_result, success))
}

/// Executes a fuzz call and returns the result.
/// Applies any block timestamp (warp) and block number (roll) adjustments before the call.
pub(crate) fn execute_tx<FEN: FoundryEvmNetwork>(
    executor: &mut Executor<FEN>,
    tx: &BasicTxDetails,
) -> Result<RawCallResult<FEN>> {
    let warp = tx.warp.unwrap_or_default();
    let roll = tx.roll.unwrap_or_default();

    if warp > 0 || roll > 0 {
        // Apply pre-call block adjustments to the executor's env.
        let ts = executor.evm_env().block_env.timestamp();
        let num = executor.evm_env().block_env.number();
        executor.evm_env_mut().block_env.set_timestamp(ts + warp);
        executor.evm_env_mut().block_env.set_number(num + roll);

        // Also update the inspector's cheatcodes.block if set.
        // The inspector's block may override the env during interpreter initialization,
        // so we need to add our warp/roll on top of any existing cheatcode-set values.
        let block_env = executor.evm_env().block_env.clone();
        if let Some(cheatcodes) = executor.inspector_mut().cheatcodes.as_mut() {
            if let Some(block) = cheatcodes.block.as_mut() {
                let bts = block.timestamp();
                let bnum = block.number();
                block.set_timestamp(bts + warp);
                block.set_number(bnum + roll);
            } else {
                cheatcodes.block = Some(block_env);
            }
        }
    }

    // Bound requested value by sender's available balance so payable paths still get
    // exercised when the requested value exceeds balance, instead of collapsing to zero.
    let requested_value = tx.call_details.value.unwrap_or(U256::ZERO);
    let sender_balance = executor.get_balance(tx.sender)?;
    let value = requested_value.min(sender_balance);
    executor
        .call_raw(tx.sender, tx.call_details.target, tx.call_details.calldata.clone(), value)
        .map_err(|e| eyre!(format!("Could not make raw evm call: {e}")))
}
