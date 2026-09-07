//! The Forge test runner.

use crate::{
    MultiContractRunner, TestFilter,
    coverage::HitMaps,
    fuzz::{BaseCounterExample, FuzzTestResult},
    multi_runner::{
        FuzzMinimizeConfig, FuzzMinimizeMode, FuzzMinimizeObservation, LibraryDeployment,
        TestContract, TestFunctionMatcher, TestRunnerConfig,
        is_generated_symbolic_regression_contract,
    },
    progress::TestsProgress,
    result::{
        InvariantFailure, InvariantOutcome, InvariantPredicateResult, SuiteResult,
        SymbolicArtifactRef, SymbolicCallTrace, SymbolicCorpusSeedMetadata, SymbolicCorpusSeedRef,
        SymbolicCounterexample, SymbolicCounterexampleArtifact, SymbolicCounterexampleArtifactKind,
        SymbolicCounterexampleCall, SymbolicCounterexampleMinimization,
        SymbolicCounterexampleReplaySemantics, SymbolicCounterexampleTestIdentity,
        SymbolicInvariantArtifactFailure, SymbolicInvariantFailureSite, SymbolicReplayMetadata,
        SymbolicReplayStatus, SymbolicResult, TestKind, TestResult, TestSetup, TestStatus,
        invariant_campaign_display_name, invariant_kind,
    },
    symbolic_minimizer::{
        MinimizedSequence, minimize_sequence_counterexample, minimize_single_call_counterexample,
    },
};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi, StateMutability};
use alloy_primitives::{
    Address, B256, Bytes, I256, Selector, U256, address, hex, keccak256,
    map::{Entry, HashMap},
};
use eyre::Result;
use foundry_common::{
    LIBRARY_DEPLOYER, TestFunctionExt, TestFunctionKind, contracts::ContractsByAddress,
};
use foundry_compilers::utils::canonicalized;
use foundry_config::{
    Config, FuzzConfig, FuzzCorpusConfig, FuzzDictionaryConfig, InlineConfig, InvariantConfig,
    SymbolicConfig,
};
use foundry_evm::{
    constants::{CALLER, MAGIC_ASSUME},
    core::{backend::DatabaseExt, evm::FoundryEvmNetwork},
    decode::{RevertDecoder, SkipReason},
    executors::{
        CallResult, EvmError, Executor, ITest, InvariantReplayOptions, MinimizationReplayInput,
        RawCallResult, ShowmapOpts, ShowmapReplayTarget, StatelessReplayTarget,
        canonical_replay_dirs,
        fuzz::FuzzedExecutor,
        invariant::{
            CheckSequenceFailureSite, CheckSequenceOptions, CheckSequenceOutcome,
            HandlerAssertionFailure, InvariantExecutor, InvariantFuzzError, ReplayErrorResult,
            check_sequence, execute_tx, execute_tx_and_register_created, replay_error,
            replay_handler_failure_sequence, replay_run,
        },
        persist_corpus_seed, read_corpus_dir, replay_corpus_to_showmap,
        replay_sequence_for_minimization, should_ignore_revert,
    },
    fuzz::{
        BasicTxDetails, CallDetails, CounterExample, FuzzFixtures, fixture_name,
        invariant::{
            FuzzRunIdentifiedContracts, InvariantContract, InvariantSettings, SenderFilters,
            is_optimization_invariant,
        },
        strategies::EvmFuzzState,
    },
    inspectors::cheatcodes::Vm::AccountAccess,
    revm::{bytecode::opcode, primitives::hardfork::SpecId},
    traces::{TraceKind, TraceRequirements, load_contracts},
};
use foundry_evm_networks::NetworkVariant;
use foundry_evm_symbolic::{
    SymbolicBranchTarget, SymbolicConcreteInput, SymbolicExecutor,
    SymbolicInvariantCounterexampleKind, SymbolicInvariantRunInput, SymbolicInvariantRunResult,
    SymbolicInvariantStep, SymbolicInvariantTarget, SymbolicRunInput, SymbolicRunResult,
    SymbolicStats, SymbolicStopReason, SymbolicStorageAssignment,
};
use itertools::Itertools;
use proptest::test_runner::{RngAlgorithm, TestError, TestRng, TestRunner};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cmp::min,
    collections::BTreeMap,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::signal;
use tracing::Span;

const FUZZ_BRANCH_FRONTIER_SCHEMA: &str = "foundry:fuzz.branch-frontiers@v1";
const FUZZ_BRANCH_FRONTIER_FILE: &str = "branch-frontiers.json";

#[derive(Deserialize)]
struct FuzzBranchFrontierArtifact {
    schema: String,
    version: u32,
    test: String,
    frontiers: Vec<FuzzBranchFrontierRecord>,
}

#[derive(Deserialize)]
struct FuzzBranchFrontierRecord {
    id: u64,
    call_index: usize,
    sequence: Vec<BasicTxDetails>,
    site: FuzzBranchFrontierSite,
    operands: FuzzBranchFrontierOperands,
}

#[derive(Deserialize)]
struct FuzzBranchFrontierSite {
    address: Address,
    pc: usize,
    opcode: u8,
}

#[derive(Deserialize)]
struct FuzzBranchFrontierOperands {
    result: bool,
}

pub(crate) struct InvariantCampaignScope<'a> {
    pub config: &'a Config,
    pub inline_config: &'a InlineConfig,
    pub contract_name: &'a str,
    pub all_override_networks: &'a [NetworkVariant],
    pub pass_network: Option<&'a NetworkVariant>,
}

struct InvariantCampaignSelection<'a> {
    matched_boolean_invariant_fns: Vec<&'a Function>,
    merge_boolean_suite: bool,
    boolean_suite_anchor: Option<&'a Function>,
    optimization_anchors: usize,
}

impl InvariantCampaignSelection<'_> {
    const fn anchor_count(&self) -> usize {
        self.optimization_anchors
            + if self.matched_boolean_invariant_fns.is_empty() {
                0
            } else if self.merge_boolean_suite {
                1
            } else {
                self.matched_boolean_invariant_fns.len()
            }
    }
}

pub(crate) fn count_runnable_invariant_campaign_anchors(
    abi: &JsonAbi,
    filter: &dyn TestFilter,
    scope: InvariantCampaignScope<'_>,
) -> usize {
    let invariant_fns = abi.functions().filter(|func| func.is_invariant_test()).collect::<Vec<_>>();
    if invariant_fns.iter().any(|func| !func.inputs.is_empty()) {
        return 0;
    }

    let functions = abi
        .functions()
        .filter(|func| filter.matches_test_function(func))
        .filter(|func| {
            function_matches_network_pass(
                scope.all_override_networks,
                scope.pass_network,
                scope.inline_config.network_for(
                    &scope.config.profile,
                    scope.contract_name,
                    &func.name,
                ),
            )
        })
        .collect::<Vec<_>>();

    select_invariant_campaigns(
        &invariant_fns,
        &functions,
        scope.config,
        scope.inline_config,
        scope.contract_name,
    )
    .anchor_count()
}

pub(crate) fn function_matches_network_pass(
    all_override_networks: &[NetworkVariant],
    pass_network: Option<&NetworkVariant>,
    func_network: Option<NetworkVariant>,
) -> bool {
    if all_override_networks.is_empty() {
        return true;
    }
    match pass_network {
        None => func_network.is_none_or(|network| !all_override_networks.contains(&network)),
        Some(target) => func_network.as_ref() == Some(target),
    }
}

pub(crate) fn inline_config_for(
    config: &Config,
    inline_config: &InlineConfig,
    contract_name: &str,
    func: Option<&Function>,
) -> Result<Config> {
    let function = func.map(|f| f.name.as_str()).unwrap_or("");
    Ok(config.merge_inline_provider(inline_config.provide(contract_name, function))?)
}

fn invariant_suite_configs_match(
    config: &Config,
    inline_config: &InlineConfig,
    contract_name: &str,
    funcs: &[&Function],
) -> bool {
    let Some((anchor, rest)) = funcs.split_first() else {
        return true;
    };
    let anchor_config = match inline_config_for(config, inline_config, contract_name, Some(anchor))
    {
        Ok(config) => config.invariant,
        Err(_) => return false,
    };
    rest.iter().all(|func| {
        inline_config_for(config, inline_config, contract_name, Some(func))
            .map(|config| config.invariant == anchor_config)
            .unwrap_or(false)
    })
}

fn select_invariant_campaigns<'a>(
    invariant_fns: &[&'a Function],
    functions: &[&'a Function],
    config: &Config,
    inline_config: &InlineConfig,
    contract_name: &str,
) -> InvariantCampaignSelection<'a> {
    let boolean_invariant_fns =
        invariant_fns.iter().copied().filter(|func| !is_optimization_invariant(func));
    let matched_boolean_invariant_fns = functions
        .iter()
        .copied()
        .filter(|func| func.is_invariant_test() && !is_optimization_invariant(func))
        .collect::<Vec<_>>();
    let optimization_anchors = functions
        .iter()
        .filter(|func| func.is_invariant_test() && is_optimization_invariant(func))
        .count();

    // The boolean invariant campaign is contract-level. Test filters only select which predicates
    // are evaluated/reported inside that campaign; they must not decide the corpus/failure
    // namespace. Use the canonical anchor when it is part of the filtered set, but preserve
    // `--mt`/`--nmt` isolation when the filter deliberately excludes it.
    let canonical_boolean_anchor = boolean_invariant_fns.into_iter().next();
    let merge_boolean_suite = !matched_boolean_invariant_fns.is_empty()
        && invariant_suite_configs_match(
            config,
            inline_config,
            contract_name,
            &matched_boolean_invariant_fns,
        );
    let boolean_suite_anchor = merge_boolean_suite
        .then(|| {
            canonical_boolean_anchor
                .filter(|anchor| matched_boolean_invariant_fns.contains(anchor))
                .or_else(|| matched_boolean_invariant_fns.first().copied())
        })
        .flatten();

    InvariantCampaignSelection {
        matched_boolean_invariant_fns,
        merge_boolean_suite,
        boolean_suite_anchor,
        optimization_anchors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_common::EmptyTestFilter;
    use foundry_config::NatSpec;

    const CONTRACT_NAME: &str = "src/Test.t.sol:InvariantTest";

    #[test]
    fn symbolic_artifact_file_name_hashes_full_identity() {
        let single = symbolic_artifact_file_name(
            "src/A.t.sol:Contract",
            "test_collision()",
            SymbolicCounterexampleArtifactKind::SingleCall,
        );
        let same_file_component_different_contract = symbolic_artifact_file_name(
            "src/B.t.sol:Contract",
            "test_collision()",
            SymbolicCounterexampleArtifactKind::SingleCall,
        );
        let same_contract_different_kind = symbolic_artifact_file_name(
            "src/A.t.sol:Contract",
            "test_collision()",
            SymbolicCounterexampleArtifactKind::Sequence,
        );

        assert_ne!(single, same_file_component_different_contract);
        assert_ne!(single, same_contract_different_kind);

        let hash = single
            .strip_prefix("test_collision__-")
            .and_then(|value| value.strip_suffix(".json"))
            .expect("file name should include sanitized value prefix and json suffix");
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn symbolic_sequence_failure_identity_includes_failure_site() {
        let outcome = |site: CheckSequenceFailureSite| CheckSequenceOutcome {
            success: false,
            replayed_entirely: false,
            reason: Some("same reason".to_string()),
            calls_count: 1,
            reverts: 0,
            failure_site: Some(site),
            sequence_assertion_failure: true,
        };
        let site = |target: u8, fingerprint: u8| CheckSequenceFailureSite::SequenceCall {
            target: Address::with_last_byte(target),
            selector: Selector::from([0, 0, 0, 1]),
            fingerprint: B256::from([fingerprint; 32]),
        };
        let expected = outcome(site(1, 1));

        assert!(same_sequence_failure(&outcome(site(1, 1)), &expected));
        assert!(!same_sequence_failure(&outcome(site(2, 1)), &expected));
        assert!(!same_sequence_failure(&outcome(site(1, 2)), &expected));
    }

    fn count_anchors(abi: &JsonAbi, inline_config: &InlineConfig) -> usize {
        let config = Config::default();
        count_runnable_invariant_campaign_anchors(
            abi,
            &EmptyTestFilter::default(),
            InvariantCampaignScope {
                config: &config,
                inline_config,
                contract_name: CONTRACT_NAME,
                all_override_networks: &[],
                pass_network: None,
            },
        )
    }

    #[test]
    fn runnable_campaign_anchor_count_merges_boolean_suite_and_counts_optimizations() {
        let abi = JsonAbi::parse([
            "function invariantOne() external",
            "function invariantTwo() external",
            "function invariantOptimizeA() external returns (int256)",
            "function invariantOptimizeB() external returns (int256)",
        ])
        .unwrap();

        assert_eq!(count_anchors(&abi, &InlineConfig::new()), 3);
    }

    #[test]
    fn runnable_campaign_anchor_count_splits_boolean_suite_when_configs_differ() {
        let abi = JsonAbi::parse([
            "function invariantOne() external",
            "function invariantTwo() external",
        ])
        .unwrap();
        let mut inline_config = InlineConfig::new();
        inline_config
            .insert(&NatSpec {
                contract: CONTRACT_NAME.to_string(),
                function: Some("invariantTwo".to_string()),
                line: "1:1".to_string(),
                docs: "forge-config: default.invariant.depth = 1".to_string(),
            })
            .unwrap();

        assert_eq!(count_anchors(&abi, &inline_config), 2);
    }

    #[test]
    fn runnable_campaign_anchor_count_splits_boolean_suite_when_corpus_weight_provenance_differs() {
        let abi = JsonAbi::parse([
            "function invariantOne() external",
            "function invariantTwo() external",
        ])
        .unwrap();
        let mut inline_config = InlineConfig::new();
        inline_config
            .insert(&NatSpec {
                contract: CONTRACT_NAME.to_string(),
                function: Some("invariantTwo".to_string()),
                line: "1:1".to_string(),
                docs: "forge-config: default.invariant.corpus_random_sequence_weight = 10"
                    .to_string(),
            })
            .unwrap();

        assert_eq!(count_anchors(&abi, &inline_config), 2);
    }

    #[test]
    fn runnable_campaign_anchor_count_respects_network_pass() {
        let abi = JsonAbi::parse(["function invariantTempoOnly() external"]).unwrap();
        let mut inline_config = InlineConfig::new();
        inline_config
            .insert(&NatSpec {
                contract: CONTRACT_NAME.to_string(),
                function: Some("invariantTempoOnly".to_string()),
                line: "1:1".to_string(),
                docs: r#"forge-config: default.networks.network = "tempo""#.to_string(),
            })
            .unwrap();
        let config = Config::default();
        let override_networks = [NetworkVariant::Tempo];

        let default_pass = count_runnable_invariant_campaign_anchors(
            &abi,
            &EmptyTestFilter::default(),
            InvariantCampaignScope {
                config: &config,
                inline_config: &inline_config,
                contract_name: CONTRACT_NAME,
                all_override_networks: &override_networks,
                pass_network: None,
            },
        );
        let tempo_pass = count_runnable_invariant_campaign_anchors(
            &abi,
            &EmptyTestFilter::default(),
            InvariantCampaignScope {
                config: &config,
                inline_config: &inline_config,
                contract_name: CONTRACT_NAME,
                all_override_networks: &override_networks,
                pass_network: Some(&NetworkVariant::Tempo),
            },
        );

        assert_eq!(default_pass, 0);
        assert_eq!(tempo_pass, 1);
    }
}

/// A type that executes all tests of a contract
pub struct ContractRunner<'a, FEN: FoundryEvmNetwork> {
    /// The name of the contract.
    name: &'a str,
    /// The data of the contract.
    contract: &'a TestContract,
    /// The EVM executor.
    executor: Executor<FEN>,
    /// Overall test run progress.
    progress: Option<&'a TestsProgress>,
    /// The handle to the tokio runtime.
    tokio_handle: tokio::runtime::Handle,
    /// The span of the contract.
    span: tracing::Span,
    /// The contract-level configuration.
    tcfg: Cow<'a, TestRunnerConfig<FEN>>,
    /// The parent runner.
    mcr: &'a MultiContractRunner<FEN>,
    /// Number of matching invariant campaign anchors in the current test pass.
    num_invariant_campaign_anchors: usize,
}

pub(crate) struct ContractRunnerContext<'a> {
    pub(crate) progress: Option<&'a TestsProgress>,
    pub(crate) tokio_handle: tokio::runtime::Handle,
    pub(crate) num_invariant_campaign_anchors: usize,
}

impl<'a, FEN: FoundryEvmNetwork> Deref for ContractRunner<'a, FEN> {
    type Target = Cow<'a, TestRunnerConfig<FEN>>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.tcfg
    }
}

impl<'a, FEN: FoundryEvmNetwork> ContractRunner<'a, FEN> {
    pub(crate) fn new(
        name: &'a str,
        contract: &'a TestContract,
        executor: Executor<FEN>,
        span: Span,
        mcr: &'a MultiContractRunner<FEN>,
        context: ContractRunnerContext<'a>,
    ) -> Self {
        Self {
            name,
            contract,
            executor,
            progress: context.progress,
            tokio_handle: context.tokio_handle,
            span,
            tcfg: Cow::Borrowed(&mcr.tcfg),
            mcr,
            num_invariant_campaign_anchors: context.num_invariant_campaign_anchors,
        }
    }

    /// Returns `true` if `func` should run in the current multi-network pass.
    ///
    /// In single-pass mode (no inline network overrides) every function passes.
    /// In multi-pass mode:
    /// - Default pass (`pass_network = None`): includes functions *without* an override annotation.
    /// - Override pass (`pass_network = Some(v)`): includes only functions annotated with `v`.
    fn function_matches_network_pass(&self, func: &Function) -> bool {
        function_matches_network_pass(
            &self.mcr.tcfg.multi_network.all_override_networks,
            self.mcr.tcfg.multi_network.pass_network.as_ref(),
            self.mcr.inline_config.network_for(&self.tcfg.config.profile, self.name, &func.name),
        )
    }

    /// Deploys the test contract inside the runner from the sending account, and optionally runs
    /// the `setUp` function on the test contract.
    pub fn setup(&mut self, call_setup: bool) -> TestSetup {
        self._setup(call_setup).unwrap_or_else(|err| {
            if err.to_string().contains("skipped") {
                TestSetup::skipped(err.to_string())
            } else {
                TestSetup::failed(err.to_string())
            }
        })
    }

    fn _setup(&mut self, call_setup: bool) -> Result<TestSetup> {
        trace!(call_setup, "setting up");

        self.apply_contract_inline_config()?;

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(self.sender, U256::MAX)?;
        self.executor.set_balance(CALLER, U256::MAX)?;

        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools.
        self.executor.set_nonce(self.sender, 1)?;

        // Deploy libraries.
        self.executor.set_balance(LIBRARY_DEPLOYER, U256::MAX)?;

        let rd = &self.mcr.revert_decoder;
        let mut result = TestSetup::default();
        let mut pending_account_diffs = Vec::new();
        match self.mcr.library_deployment {
            LibraryDeployment::Nonce => {
                for (nonce, code) in self.mcr.libs_to_deploy.iter().enumerate() {
                    // Libraries are linked from nonce zero in the same order they are deployed.
                    let expected_address = LIBRARY_DEPLOYER.create(nonce as u64);
                    let (deploy_result, recorded_account_diffs) =
                        self.deploy_library(expected_address, |executor| {
                            executor.deploy(LIBRARY_DEPLOYER, code.clone(), U256::ZERO, Some(rd))
                        });

                    if let Ok(deployed) = &deploy_result {
                        result.deployed_libs.push(deployed.address);
                        if self.contract.library_addresses.contains(&deployed.address) {
                            pending_account_diffs.extend(recorded_account_diffs);
                        }
                    }

                    let (raw, reason) =
                        RawCallResult::from_evm_result(deploy_result.map(Into::into))?;
                    result.extend(raw, TraceKind::Deployment);
                    if reason.is_some() {
                        debug!(?reason, "deployment of library failed");
                        result.reason = reason;
                        return Ok(result);
                    }
                }
            }
            LibraryDeployment::Create2 { deployer, salt } => {
                // Foundry only knows how to install the canonical factory locally. A custom
                // factory is usable only when it already exists in fork state. Tempo also
                // provides the factory as a predeploy, which must not be deployed again.
                if deployer == foundry_evm::constants::DEFAULT_CREATE2_DEPLOYER
                    && !self.evm_opts.networks.is_tempo()
                {
                    self.executor.deploy_create2_deployer()?;
                }
                for code in &self.mcr.libs_to_deploy {
                    let address = deployer.create2_from_code(salt, code);
                    if self.executor.is_empty_code(address)? {
                        let calldata = [salt.as_slice(), code.as_ref()].concat().into();
                        let (raw, recorded_account_diffs) =
                            self.deploy_library(address, |executor| {
                                executor.transact_raw(
                                    LIBRARY_DEPLOYER,
                                    deployer,
                                    calldata,
                                    U256::ZERO,
                                )
                            });
                        let raw = raw?;
                        let (raw, reason) = if raw.reverted {
                            RawCallResult::from_evm_result(Err(raw.into_evm_error(Some(rd))))?
                        } else {
                            (raw, None)
                        };
                        result.extend(raw, TraceKind::Deployment);
                        if reason.is_some() {
                            debug!(?reason, "CREATE2 deployment of library failed");
                            result.reason = reason;
                            return Ok(result);
                        }
                        if self.executor.is_empty_code(address)? {
                            result.reason = Some(format!(
                                "CREATE2 library deployment succeeded but no code was found at {address}"
                            ));
                            return Ok(result);
                        }
                        pending_account_diffs.extend(recorded_account_diffs);
                    }
                    self.executor.backend_mut().add_persistent_account(address);
                    result.deployed_libs.push(address);
                }

                // Factory calls are test harness setup and must not be observable through the
                // last-call gas cheatcodes.
                if let Some(cheats) = self.executor.inspector_mut().cheatcodes.as_mut() {
                    cheats.gas_metering.last_call_gas = None;
                    cheats.gas_metering.last_frame_gas = None;
                }
            }
        }
        if !pending_account_diffs.is_empty()
            && let Some(cheats) = self.executor.inspector_mut().cheatcodes.as_deref_mut()
        {
            cheats.set_pending_account_diffs(pending_account_diffs);
        }

        // Configured libraries may already exist and are not present in `libs_to_deploy`.
        for &address in &self.mcr.library_addresses {
            if !self.executor.is_empty_code(address)? {
                result.deployed_libs.push(address);
            }
        }
        result.deployed_libs.sort_unstable();
        result.deployed_libs.dedup();

        let address = self.sender.create(self.executor.get_nonce(self.sender)?);
        result.address = address;

        // Set the contracts initial balance before deployment, so it is available during
        // construction
        self.executor.set_balance(address, self.initial_balance())?;

        // Deploy the test contract
        let deploy_result =
            self.executor.deploy(self.sender, self.contract.bytecode.clone(), U256::ZERO, Some(rd));

        result.deployment_failure = deploy_result.is_err();

        if let Ok(dr) = &deploy_result {
            debug_assert_eq!(dr.address, address);
        }
        let (raw, reason) = RawCallResult::from_evm_result(deploy_result.map(Into::into))?;
        result.extend(raw, TraceKind::Deployment);
        if reason.is_some() {
            debug!(?reason, "deployment of test contract failed");
            result.reason = reason;
            return Ok(result);
        }

        // Reset `self.sender`s, `CALLER`s and `LIBRARY_DEPLOYER`'s balance to the initial balance.
        self.executor.set_balance(self.sender, self.initial_balance())?;
        self.executor.set_balance(CALLER, self.initial_balance())?;
        self.executor.set_balance(LIBRARY_DEPLOYER, self.initial_balance())?;

        if matches!(self.mcr.library_deployment, LibraryDeployment::Nonce)
            && !self.evm_opts.networks.is_tempo()
        {
            self.executor.deploy_create2_deployer()?;
        }

        // Optionally call the `setUp` function
        if call_setup {
            trace!("calling setUp");
            let res = self.executor.setup(None, address, Some(rd));
            let (raw, reason) = RawCallResult::from_evm_result(res)?;
            result.extend(raw, TraceKind::Setup);
            result.reason = reason;
        }

        Ok(result)
    }

    fn initial_balance(&self) -> U256 {
        self.evm_opts.initial_balance
    }

    /// Runs `deploy`, recording the account diffs of linked library deployments so cheatcodes
    /// can attribute them to the library.
    fn deploy_library<T>(
        &mut self,
        address: Address,
        deploy: impl FnOnce(&mut Executor<FEN>) -> T,
    ) -> (T, Vec<AccountAccess>) {
        let recording = self.contract.library_addresses.contains(&address)
            && self
                .executor
                .inspector_mut()
                .cheatcodes
                .as_deref_mut()
                .is_some_and(|cheats| cheats.start_internal_state_diff_recording());
        let result = deploy(&mut self.executor);
        let diffs = if recording {
            self.executor
                .inspector_mut()
                .cheatcodes
                .as_deref_mut()
                .map(|cheats| cheats.stop_internal_state_diff_recording())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        (result, diffs)
    }

    /// Configures this runner with the inline configuration for the contract.
    fn apply_contract_inline_config(&mut self) -> Result<()> {
        if self.inline_config.contains_contract(self.name) {
            let new_config = Arc::new(self.inline_config(None)?);
            self.tcfg.to_mut().reconfigure_with(new_config);
            let prev_tracer = self.executor.inspector_mut().tracer.take();
            self.tcfg.configure_executor(&mut self.executor);
            // Don't set tracer here.
            self.executor.inspector_mut().tracer = prev_tracer;
        }
        Ok(())
    }

    /// Returns the configuration for a contract or function.
    fn inline_config(&self, func: Option<&Function>) -> Result<Config> {
        let mut config = inline_config_for(&self.config, &self.mcr.inline_config, self.name, func)?;
        config.networks = config.networks.with_execution_profile(self.tcfg.evm_opts.networks);
        Ok(config)
    }

    /// Collect fixtures from test contract.
    ///
    /// Fixtures can be defined:
    /// - as storage arrays in test contract, prefixed with `fixture`
    /// - as functions prefixed with `fixture` and followed by parameter name to be fuzzed
    ///
    /// Storage array fixtures:
    /// `uint256[] public fixture_amount = [1, 2, 3];`
    /// define an array of uint256 values to be used for fuzzing `amount` named parameter in scope
    /// of the current test.
    ///
    /// Function fixtures:
    /// `function fixture_owner() public returns (address[] memory){}`
    /// returns an array of addresses to be used for fuzzing `owner` named parameter in scope of the
    /// current test.
    fn fuzz_fixtures(&mut self, address: Address) -> FuzzFixtures {
        let mut fixtures = HashMap::default();
        let fixture_functions = self.contract.abi.functions().filter(|func| func.is_fixture());
        for func in fixture_functions {
            if func.inputs.is_empty() {
                // Read fixtures declared as functions.
                if let Ok(CallResult { raw: _, decoded_result }) =
                    self.executor.call(CALLER, address, func, &[], U256::ZERO, None)
                {
                    fixtures.insert(fixture_name(func.name.clone()), decoded_result);
                }
            } else {
                // For reading fixtures from storage arrays we collect values by calling the
                // function with incremented indexes until there's an error.
                let mut vals = Vec::new();
                let mut index = 0;
                loop {
                    if let Ok(CallResult { raw: _, decoded_result }) = self.executor.call(
                        CALLER,
                        address,
                        func,
                        &[DynSolValue::Uint(U256::from(index), 256)],
                        U256::ZERO,
                        None,
                    ) {
                        vals.push(decoded_result);
                    } else {
                        // No result returned for this index, we reached the end of storage
                        // array or the function is not a valid fixture.
                        break;
                    }
                    index += 1;
                }
                fixtures.insert(fixture_name(func.name.clone()), DynSolValue::Array(vals));
            };
        }
        FuzzFixtures::new(fixtures).with_enum_bounds(self.mcr.enum_bounds.clone())
    }

    /// Classifies test functions with the current contract-level configuration.
    fn test_matcher(&self) -> TestFunctionMatcher<'_> {
        TestFunctionMatcher::new(
            &self.config,
            &self.mcr.inline_config,
            self.mcr.tcfg.symbolic_artifact_replay.as_ref(),
        )
    }

    /// Returns the test functions selected by `filter` that run in the current network pass.
    fn matching_test_functions(
        &self,
        filter: &dyn TestFilter,
        test_matcher: &TestFunctionMatcher<'_>,
    ) -> Vec<&'a Function> {
        test_matcher
            .test_functions(self.name.to_string(), &self.contract.abi, |contract_id, func, kind| {
                filter.matches_test_function_kind_in_contract(contract_id, func, kind)
                    && self.function_matches_network_pass(func)
            })
            .collect()
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(mut self, filter: &dyn TestFilter) -> SuiteResult {
        let start = Instant::now();
        let mut warnings = Vec::new();
        let generated_symbolic_regression =
            is_generated_symbolic_regression_contract(&self.contract.abi);
        // Classified before `setUp`; the full function list is built after setup so
        // contract-level inline config can still affect symbolic entrypoint discovery.
        let test_matcher = self.test_matcher();
        // In fuzz-only mode, drop suites with no runnable fuzz or invariant tests before
        // executing `setUp`.
        if self.mcr.tcfg.fuzz_only
            && !self.matching_test_functions(filter, &test_matcher).into_iter().any(|func| {
                matches!(
                    test_matcher.test_function_kind(self.name, func, generated_symbolic_regression),
                    TestFunctionKind::FuzzTest { .. } | TestFunctionKind::InvariantTest
                )
            })
        {
            return SuiteResult::new(start.elapsed(), BTreeMap::new(), warnings);
        }

        // Check if `setUp` function with valid signature declared.
        let setup_fns: Vec<_> =
            self.contract.abi.functions().filter(|func| func.name.is_setup()).collect();
        let call_setup = setup_fns.len() == 1 && setup_fns[0].name == "setUp";
        // There is a single miss-cased `setUp` function, so we add a warning
        for &setup_fn in &setup_fns {
            if setup_fn.name != "setUp" {
                warnings.push(format!(
                    "Found invalid setup function \"{}\" did you mean \"setUp()\"?",
                    setup_fn.signature()
                ));
            }
        }

        // There are multiple setUp function, so we return a single test result for `setUp`
        if setup_fns.len() > 1 {
            return self.failed_suite(
                start,
                warnings,
                [("setUp()".to_string(), TestResult::fail("multiple setUp functions".to_string()))],
            );
        }

        // Check if `afterInvariant` function with valid signature declared.
        let after_invariant_fns: Vec<_> =
            self.contract.abi.functions().filter(|func| func.name.is_after_invariant()).collect();
        if after_invariant_fns.len() > 1 {
            return self.failed_suite(
                start,
                warnings,
                [(
                    "afterInvariant()".to_string(),
                    TestResult::fail("multiple afterInvariant functions".to_string()),
                )],
            );
        }
        let call_after_invariant = after_invariant_fns.first().is_some_and(|after_invariant_fn| {
            let match_sig = after_invariant_fn.name == "afterInvariant";
            if !match_sig {
                warnings.push(format!(
                    "Found invalid afterInvariant function \"{}\" did you mean \"afterInvariant()\"?",
                    after_invariant_fn.signature()
                ));
            }
            match_sig
        });

        let invariant_fns = self
            .contract
            .abi
            .functions()
            .filter(|func| {
                test_matcher
                    .test_function_kind(self.name, func, generated_symbolic_regression)
                    .is_invariant_test()
            })
            .collect::<Vec<_>>();

        // Validate signatures up front: invariant functions must take no parameters. Without
        // this, parameterized `invariant_*` functions would slip into contract-level campaigns
        // and fail with a confusing "selector not found" / decode error mid-campaign. Reject
        // here with a per-function result so the failure is obvious to the user.
        let invalid_invariants = invariant_fns
            .iter()
            .filter(|f| !f.inputs.is_empty())
            .map(|f| {
                let signature = f.signature();
                let reason = format!("invariant `{signature}` must take no parameters");
                (signature, TestResult::fail(reason))
            })
            .collect::<Vec<_>>();
        if !invalid_invariants.is_empty() {
            return self.failed_suite(start, warnings, invalid_invariants);
        }

        for invariant in &invariant_fns {
            if invariant.outputs.len() == 1 && invariant.outputs[0].ty == "bool" {
                warnings.push(format!(
                    "Invariant function `{}` returns `bool`, but its return value is ignored; use assertions or revert to indicate failure.",
                    invariant.signature()
                ));
            }
        }

        // Invariant testing requires tracing to figure out what contracts were created.
        // For regular test runs we disable debug-level setup traces as an optimization.
        // In `forge test --debug`, keep setup traces in debug mode so setup failures are
        // inspectable in the debugger.
        let has_invariants = !invariant_fns.is_empty();

        let should_override_setup_tracing =
            !self.tcfg.debug && (self.executor.inspector().tracer.is_some() || has_invariants);

        let prev_tracer = should_override_setup_tracing.then(|| {
            let prev_tracer = self.executor.inspector_mut().tracer.take();
            self.executor.set_trace_requirements(TraceRequirements::none().with_calls(true));
            prev_tracer
        });

        let setup_time = Instant::now();
        let mut setup = self.setup(call_setup);
        debug!("finished setting up in {:?}", setup_time.elapsed());

        if let Some(prev_tracer) = prev_tracer {
            self.executor.inspector_mut().tracer = prev_tracer;
        }

        if setup.reason.is_some() {
            // The setup failed, so we return a single test result for `setUp`
            let name = if setup.deployment_failure { "constructor()" } else { "setUp()" };
            return self.failed_suite(
                start,
                warnings,
                [(name.to_string(), TestResult::setup_result(setup))],
            );
        }

        // Filter out functions sequentially since it's very fast and there is no need to do it
        // in parallel.
        let find_timer = Instant::now();
        let functions = self.matching_test_functions(filter, &self.test_matcher());
        debug!(
            "Found {} test functions out of {} in {:?}",
            functions.len(),
            self.contract.abi.functions().count(),
            find_timer.elapsed(),
        );

        let identified_contracts = has_invariants.then(|| {
            load_contracts(setup.traces.iter().map(|(_, t)| &t.arena), &self.mcr.known_contracts)
        });

        if let Some(replay) = &self.mcr.tcfg.symbolic_artifact_replay {
            let artifact = &replay.artifact;
            let target = &artifact.test;
            let replay_functions =
                functions.iter().filter(|func| func.signature() == target.test).collect::<Vec<_>>();
            let func = match replay_functions[..] {
                [] if !self.mcr.tcfg.multi_network.all_override_networks.is_empty() => {
                    return SuiteResult::new(start.elapsed(), BTreeMap::new(), warnings);
                }
                [] => {
                    let reason = format!(
                        "symbolic artifact target `{}` was not found in `{}`",
                        target.test, target.contract
                    );
                    let results = [(target.test.clone(), TestResult::fail(reason))];
                    return SuiteResult::new(start.elapsed(), results.into(), warnings);
                }
                [func] => *func,
                _ => {
                    let reason = format!(
                        "symbolic artifact target `{}` matched {} functions in `{}`",
                        target.test,
                        replay_functions.len(),
                        target.contract
                    );
                    let results = [(target.test.clone(), TestResult::fail(reason))];
                    return SuiteResult::new(start.elapsed(), results.into(), warnings);
                }
            };

            let is_sequence = artifact.kind == SymbolicCounterexampleArtifactKind::Sequence;
            let kind = if is_sequence {
                func.test_function_kind()
            } else {
                TestFunctionKind::SymbolicTest
            };
            let test_start = Instant::now();
            let mut res = if is_sequence && !kind.is_invariant_test() {
                TestResult::fail(format!(
                    "sequence symbolic artifact must target an invariant test, but matched {} function `{}`",
                    kind.name(),
                    func.signature(),
                ))
            } else {
                let invariants = if is_sequence { std::slice::from_ref(&func) } else { &[][..] };
                FunctionRunner::new(&self, &setup).run_symbolic_artifact_replay(
                    func,
                    invariants,
                    call_after_invariant,
                )
            };
            res.duration = test_start.elapsed();
            debug!(%kind, path = %replay.path.display(), "replayed symbolic artifact");
            return SuiteResult::new(start.elapsed(), [(func.signature(), res)].into(), warnings);
        }

        let test_fail_results = functions
            .iter()
            .filter(|func| func.test_function_kind().is_any_test_fail())
            .map(|func| {
                let reason = "`testFail*` has been removed. Consider changing to test_Revert[If|When]_Condition and expecting a revert";
                (func.signature(), TestResult::fail(reason.to_string()))
            })
            .collect::<Vec<_>>();
        if !test_fail_results.is_empty() {
            return self.failed_suite(start, warnings, test_fail_results);
        }

        if functions.iter().any(|func| {
            matches!(
                func.test_function_kind(),
                TestFunctionKind::FuzzTest { .. }
                    | TestFunctionKind::TableTest
                    | TestFunctionKind::InvariantTest
            )
        }) {
            setup.fuzz_fixtures = self.fuzz_fixtures(setup.address);
        }

        let early_exit = &self.tcfg.early_exit;
        let test_matcher = self.test_matcher();
        if self.progress.is_some() {
            let interrupt = early_exit.clone();
            self.tokio_handle.spawn(async move {
                signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
                interrupt.record_ctrl_c();
            });
        }

        let InvariantCampaignSelection {
            matched_boolean_invariant_fns,
            merge_boolean_suite: merge_invariant_suite,
            boolean_suite_anchor: invariant_suite_anchor,
            optimization_anchors: _,
        } = select_invariant_campaigns(
            &invariant_fns,
            &functions,
            &self.config,
            &self.mcr.inline_config,
            self.name,
        );

        let test_results = functions
            .par_iter()
            .filter_map(|&func| {
                // Early exit if we're running with fail-fast and a test already failed.
                if early_exit.should_stop() {
                    return None;
                }
                // Invariant tests run either as a shared boolean suite or as a single
                // optimization campaign; other test kinds keep their original invariant set.
                let invariants: &[&Function] = if func.is_invariant_test() {
                    if is_optimization_invariant(func) {
                        std::slice::from_ref(&func)
                    } else if merge_invariant_suite {
                        // Only the suite anchor runs the merged boolean campaign.
                        if invariant_suite_anchor != Some(func) {
                            return None;
                        }
                        matched_boolean_invariant_fns.as_slice()
                    } else {
                        std::slice::from_ref(&func)
                    }
                } else {
                    invariant_fns.as_slice()
                };

                // Skip invariant anchors that have no predicates to execute.
                if func.is_invariant_test() && invariants.is_empty() {
                    return None;
                }

                let start = Instant::now();

                let _guard = self.tokio_handle.enter();

                let _guard;
                let current_span = tracing::Span::current();
                if current_span.is_none() || current_span.id() != self.span.id() {
                    _guard = self.span.enter();
                }

                let sig = func.signature();
                let kind =
                    test_matcher.test_function_kind(self.name, func, generated_symbolic_regression);

                let _guard = debug_span!(
                    "test",
                    %kind,
                    name = %if enabled!(tracing::Level::TRACE) { &sig } else { &func.name },
                )
                .entered();

                let mut res = FunctionRunner::new(&self, &setup).run(
                    func,
                    invariants,
                    kind,
                    call_after_invariant,
                    identified_contracts.as_ref(),
                );
                res.duration = start.elapsed();

                // Record test failure for early exit (only triggers if fail-fast is enabled).
                if res.status.is_failure() {
                    early_exit.record_failure();
                }

                Some((sig, res))
            })
            .collect::<BTreeMap<_, _>>();

        SuiteResult::new(start.elapsed(), test_results, warnings)
    }

    /// Returns a suite that failed before its tests could run, tripping the global fail-fast
    /// flag so sibling parallel suites (notably long-running invariant campaigns) observe
    /// `should_stop()` and exit at their next run boundary instead of running to their timeout.
    fn failed_suite(
        &self,
        start: Instant,
        warnings: Vec<String>,
        results: impl IntoIterator<Item = (String, TestResult)>,
    ) -> SuiteResult {
        self.tcfg.early_exit.record_failure();
        SuiteResult::new(start.elapsed(), results.into_iter().collect(), warnings)
    }
}

/// Executes a single test function, returning a [`TestResult`].
struct FunctionRunner<'a, FEN: FoundryEvmNetwork> {
    /// The function-level configuration.
    tcfg: Cow<'a, TestRunnerConfig<FEN>>,
    /// The EVM executor.
    executor: Cow<'a, Executor<FEN>>,
    /// The parent runner.
    cr: &'a ContractRunner<'a, FEN>,
    /// The address of the test contract.
    address: Address,
    /// The test setup result.
    setup: &'a TestSetup,
    /// The test result. Returned after running the test.
    result: TestResult,
}

/// A replayed and shrunk invariant counterexample.
struct ReplayedInvariantSequence {
    call_sequence: Vec<BaseCounterExample>,
    artifact: Option<SymbolicArtifactRef>,
    minimization: Option<SymbolicCounterexampleMinimization>,
    fork_block_number: Option<u64>,
}

/// A stateful call sequence replay target shared by symbolic minimization and failure checks.
#[derive(Clone, Copy)]
struct SequenceReplay<'a> {
    invariant_config: &'a InvariantConfig,
    invariant_contract: &'a InvariantContract<'a>,
    target_invariant: &'a Function,
    assertion_failure: bool,
    storage: &'a [SymbolicStorageAssignment],
}

/// Metadata for the symbolic artifact persisted with a replayed invariant sequence.
struct SequenceArtifactSpec<'a> {
    file_name: &'a str,
    fail_on_revert: bool,
    failure: Option<SymbolicInvariantArtifactFailure>,
}

/// Returns `true` if two sequence replays failed in the same way at the same site.
fn same_sequence_failure(actual: &CheckSequenceOutcome, expected: &CheckSequenceOutcome) -> bool {
    actual.replayed_entirely == expected.replayed_entirely
        && actual.reason == expected.reason
        && actual.failure_site == expected.failure_site
        && actual.sequence_assertion_failure == expected.sequence_assertion_failure
}

impl<'a, FEN: FoundryEvmNetwork> Deref for FunctionRunner<'a, FEN> {
    type Target = Cow<'a, TestRunnerConfig<FEN>>;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.tcfg
    }
}

impl<'a, FEN: FoundryEvmNetwork> FunctionRunner<'a, FEN> {
    fn new(cr: &'a ContractRunner<'a, FEN>, setup: &'a TestSetup) -> Self {
        Self {
            tcfg: Cow::Borrowed(cr.tcfg.as_ref()),
            executor: Cow::Borrowed(&cr.executor),
            cr,
            address: setup.address,
            setup,
            result: TestResult::new(setup),
        }
    }

    const fn revert_decoder(&self) -> &'a RevertDecoder {
        &self.cr.mcr.revert_decoder
    }

    /// Creates the progress bar for a fuzz or invariant campaign, if progress is shown.
    fn fuzz_progress(
        &self,
        test_name: &str,
        timeout: Option<u32>,
        runs: u32,
    ) -> Option<indicatif::ProgressBar> {
        self.cr.progress?.inner.lock().start_fuzz_progress(self.cr.name, test_name, timeout, runs)
    }

    fn fuzz_minimize_target_id(&self, test_name: &str) -> String {
        let network = self
            .cr
            .mcr
            .tcfg
            .multi_network
            .pass_network
            .as_ref()
            .map(|network| format!("{network:?}"))
            .unwrap_or_else(|| "default".to_string());
        format!("{network}:{}::{test_name}", self.cr.name)
    }

    /// Builds a symbolic counterexample artifact for this test.
    fn symbolic_artifact(
        &self,
        test_name: &str,
        kind: SymbolicCounterexampleArtifactKind,
        symbolic: &SymbolicResult,
        fail_on_revert: bool,
        calls: Vec<SymbolicCounterexampleCall>,
    ) -> SymbolicCounterexampleArtifact {
        SymbolicCounterexampleArtifact::new(
            kind,
            SymbolicCounterexampleTestIdentity {
                contract: self.cr.name.to_string(),
                test: test_name.to_string(),
            },
            symbolic,
            SymbolicCounterexampleReplaySemantics { fail_on_revert },
            calls,
        )
    }

    /// Writes `artifact` to the stable per-test path, so the latest counterexample replaces
    /// older ones, and returns a reference to it.
    fn write_symbolic_artifact(
        &self,
        file_name: &str,
        artifact: &SymbolicCounterexampleArtifact,
    ) -> Option<SymbolicArtifactRef> {
        let dir = self
            .config
            .cache_path
            .join("symbolic")
            .join(sanitize_symbolic_artifact_component(self.cr.name));
        let path = dir.join(symbolic_artifact_file_name(self.cr.name, file_name, artifact.kind));
        if let Err(err) = foundry_common::fs::create_dir_all(&dir) {
            tracing::error!(%err, path = %dir.display(), "Failed to create symbolic artifact dir");
            return None;
        }
        if let Err(err) = foundry_common::fs::write_json_file(&path, artifact) {
            tracing::error!(%err, path = %path.display(), "Failed to write symbolic artifact");
            return None;
        }
        Some(SymbolicArtifactRef::new(path))
    }

    /// Persists a replay-confirmed stateful counterexample as a sequence artifact.
    fn persist_sequence_artifact(
        &self,
        test_name: &str,
        file_name: &str,
        calls: Vec<SymbolicCounterexampleCall>,
        fail_on_revert: bool,
        storage: &[SymbolicStorageAssignment],
        failure: Option<SymbolicInvariantArtifactFailure>,
    ) -> Option<SymbolicArtifactRef> {
        if calls.is_empty() || !self.config.symbolic.enabled {
            return None;
        }
        let symbolic = SymbolicResult::incomplete(
            &self.config.symbolic,
            SymbolicStopReason::Error,
            "concrete replay confirmed stateful counterexample",
            SymbolicStats::default(),
            SymbolicReplayMetadata::confirmed(),
            SymbolicCallTrace::none(),
            None,
        );
        let mut artifact = self.symbolic_artifact(
            test_name,
            SymbolicCounterexampleArtifactKind::Sequence,
            &symbolic,
            fail_on_revert,
            calls,
        );
        if !storage.is_empty() {
            artifact = artifact.with_storage(storage.to_vec());
        }
        if let Some(failure) = failure {
            artifact = artifact.with_invariant_failure(failure);
        }
        self.write_symbolic_artifact(file_name, &artifact)
    }

    /// Converts a counterexample sequence into artifact calls.
    fn sequence_calls(
        &self,
        call_sequence: &[BaseCounterExample],
    ) -> Vec<SymbolicCounterexampleCall> {
        call_sequence
            .iter()
            .map(|counterexample| {
                SymbolicCounterexampleCall::from_base_counterexample(
                    counterexample,
                    CALLER,
                    self.address,
                )
            })
            .collect()
    }

    /// Replays a single-call minimization candidate and checks it fails for `expected_reason`.
    fn replay_confirmed_symbolic_single_call(
        &self,
        call: &SymbolicCounterexampleCall,
        expected_reason: Option<&str>,
    ) -> Result<(RawCallResult<FEN>, Option<String>), String> {
        let Some(expected_reason) = expected_reason else {
            return Err("candidate replay has no stable failure reason to compare".to_string());
        };

        let mut executor = self.clone_executor();
        let raw = execute_tx(&mut executor, &call.to_basic_tx_details())
            .map_err(|err| err.to_string())?;
        if executor.is_raw_call_success(
            self.address,
            Cow::Borrowed(&raw.state_changeset),
            &raw,
            false,
        ) {
            return Err("candidate replay succeeded".to_string());
        }
        if let Some(reason) = raw.skip_reason() {
            return Err(format!("vm.skip during concrete replay: {reason}"));
        }

        let reason = (raw.reverted || raw.exit_reason.is_some_and(|reason| !reason.is_ok()))
            .then(|| self.revert_decoder().decode(&raw.result, raw.exit_reason));
        if reason.as_deref() != Some(expected_reason) {
            return Err(format!(
                "candidate replay failed with different reason: expected `{expected_reason}`, got `{}`",
                reason.as_deref().unwrap_or("")
            ));
        }
        Ok((raw, reason))
    }

    /// Shrinks and replays a failing invariant call sequence, persisting the confirmed
    /// counterexample as a symbolic artifact.
    #[expect(clippy::too_many_arguments)]
    fn replay_invariant_error_sequence(
        &mut self,
        replay: SequenceReplay<'_>,
        original_calls: &[BasicTxDetails],
        inner_sequence: Option<Vec<Option<BasicTxDetails>>>,
        identified_contracts: &ContractsByAddress,
        current_settings: &InvariantSettings,
        artifact: SequenceArtifactSpec<'_>,
        progress: Option<&indicatif::ProgressBar>,
        position: Option<(usize, usize)>,
    ) -> Result<ReplayedInvariantSequence> {
        let minimization = self.minimize_symbolic_invariant_sequence(
            replay,
            original_calls,
            identified_contracts,
            current_settings,
        );

        let mut replay_config = replay.invariant_config.clone();
        let minimized_txes;
        let replay_calls = if let Some(minimization) = &minimization {
            minimized_txes = minimization
                .minimized_calls
                .iter()
                .map(SymbolicCounterexampleCall::to_basic_tx_details)
                .collect::<Vec<_>>();
            replay_config.shrink_run_limit = 0;
            minimized_txes.as_slice()
        } else {
            original_calls
        };

        let ReplayErrorResult { counterexample_sequence: call_sequence, fork_block_number, .. } =
            self.replay_error(
                replay_config,
                self.clone_executor_with_symbolic_storage(replay.storage)?,
                replay_calls,
                inner_sequence,
                replay.assertion_failure,
                None,
                replay.invariant_contract,
                replay.target_invariant,
                identified_contracts,
                progress,
                position,
            )?;

        let test_name = replay.target_invariant.signature();
        let calls = self.sequence_calls(&call_sequence);
        let (artifact_ref, minimization) = match minimization {
            None => (
                self.persist_sequence_artifact(
                    &test_name,
                    artifact.file_name,
                    calls,
                    artifact.fail_on_revert,
                    replay.storage,
                    artifact.failure,
                ),
                None,
            ),
            Some(minimization) => {
                let original = self.persist_sequence_artifact(
                    &test_name,
                    &format!("original__{}", artifact.file_name),
                    minimization.original_calls.clone(),
                    artifact.fail_on_revert,
                    replay.storage,
                    artifact.failure.clone(),
                );
                let minimized = self.persist_sequence_artifact(
                    &test_name,
                    artifact.file_name,
                    calls,
                    artifact.fail_on_revert,
                    replay.storage,
                    artifact.failure,
                );
                // Schema v1 cannot persist an empty sequence; retain the confirmed original
                // artifact.
                let primary = if call_sequence.is_empty() { &original } else { &minimized };
                let primary = primary.clone();
                let metadata = original.zip(minimized).map(|(original, minimized)| {
                    SymbolicCounterexampleMinimization::new(
                        original,
                        minimized,
                        minimization.attempts,
                        minimization.accepted,
                        minimization.original_calldata_bytes(),
                        minimization.minimized_calldata_bytes(),
                    )
                    .with_sequence_lengths(
                        minimization.original_calls.len(),
                        minimization.minimized_calls.len(),
                    )
                });
                (primary, metadata)
            }
        };

        Ok(ReplayedInvariantSequence {
            call_sequence,
            artifact: artifact_ref,
            minimization,
            fork_block_number,
        })
    }

    /// Shrinks and replays a failing call sequence, collecting logs, traces and coverage into
    /// the test result. Returns the counterexample, the terminal check outcome when shrinking
    /// re-checked the sequence, and the fork block number.
    #[expect(clippy::too_many_arguments)]
    fn replay_error(
        &mut self,
        config: InvariantConfig,
        executor: Executor<FEN>,
        calls: &[BasicTxDetails],
        inner_sequence: Option<Vec<Option<BasicTxDetails>>>,
        expect_assertion_failure: bool,
        target_value: Option<I256>,
        invariant_contract: &InvariantContract<'_>,
        target_invariant: &Function,
        identified_contracts: &ContractsByAddress,
        progress: Option<&indicatif::ProgressBar>,
        position: Option<(usize, usize)>,
    ) -> Result<ReplayErrorResult> {
        replay_error(
            config,
            executor,
            calls,
            inner_sequence,
            expect_assertion_failure,
            target_value.is_none().then(|| self.revert_decoder()),
            target_value,
            invariant_contract,
            target_invariant,
            &self.cr.mcr.known_contracts,
            identified_contracts.clone(),
            &mut self.result.logs,
            &mut self.result.traces,
            &mut self.result.debug_bytecodes,
            &mut self.result.line_coverage,
            &mut self.result.deprecated_cheatcodes,
            progress,
            &self.tcfg.early_exit,
            position,
        )
    }

    fn minimize_symbolic_invariant_sequence(
        &self,
        replay: SequenceReplay<'_>,
        calls: &[BasicTxDetails],
        identified_contracts: &ContractsByAddress,
        current_settings: &InvariantSettings,
    ) -> Option<MinimizedSequence> {
        if !self.config.symbolic.enabled || calls.is_empty() {
            return None;
        }

        let original_calls = self.sequence_calls(&base_counterexamples(
            calls,
            identified_contracts,
            replay.invariant_config.show_solidity,
        ));
        let expected = self.symbolic_sequence_failure(replay, &original_calls)?;
        let preserves = |candidate: &[SymbolicCounterexampleCall]| {
            self.symbolic_sequence_failure(replay, candidate)
                .is_some_and(|actual| same_sequence_failure(&actual, &expected))
        };

        let minimization = minimize_sequence_counterexample(
            &original_calls,
            &self.symbolic_sequence_sender_candidates(current_settings),
            replay.invariant_config.shrink_run_limit as usize,
            preserves,
        )?;
        preserves(&minimization.minimized_calls).then_some(minimization)
    }

    fn symbolic_sequence_sender_candidates(
        &self,
        current_settings: &InvariantSettings,
    ) -> Vec<Address> {
        let mut candidates = if current_settings.target_senders.is_empty() {
            vec![self.sender, CALLER, address!("0x0000000000000000000000000000000000000100")]
        } else {
            current_settings.target_senders.clone()
        };

        candidates.retain(|sender| {
            !current_settings.excluded_senders.contains(sender)
                && (current_settings.target_senders.is_empty()
                    || current_settings.target_senders.contains(sender))
        });
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    /// Replays `calls` concretely and returns the outcome if the sequence still fails.
    fn symbolic_sequence_failure(
        &self,
        replay: SequenceReplay<'_>,
        calls: &[SymbolicCounterexampleCall],
    ) -> Option<CheckSequenceOutcome> {
        let txes =
            calls.iter().map(SymbolicCounterexampleCall::to_basic_tx_details).collect::<Vec<_>>();
        let sequence = (0..txes.len()).collect::<Vec<_>>();
        let outcome = check_sequence(
            self.clone_executor_with_symbolic_storage(replay.storage).ok()?,
            &txes,
            &sequence,
            replay.invariant_contract.address,
            replay.target_invariant.selector().to_vec().into(),
            CheckSequenceOptions {
                accumulate_warp_roll: false,
                fail_on_revert: replay.invariant_config.fail_on_revert,
                expect_assertion_failure: replay.assertion_failure,
                call_after_invariant: replay.invariant_contract.call_after_invariant,
                rd: Some(self.revert_decoder()),
            },
        )
        .ok()?;
        (!outcome.success).then_some(outcome)
    }

    /// Converts a persisted counterexample into transactions (applying `show_solidity` in
    /// place) and replays it through `check_sequence`.
    fn replay_persisted_call_sequence(
        &self,
        invariant_contract: &InvariantContract<'_>,
        call_sequence: &mut [BaseCounterExample],
        expect_assertion_failure: bool,
        storage: &[SymbolicStorageAssignment],
    ) -> Result<(Vec<BasicTxDetails>, CheckSequenceOutcome)> {
        let config = &self.config.invariant;
        let txes = base_counterexamples_to_txes(call_sequence, config.show_solidity);
        let sequence = (0..min(txes.len(), config.depth as usize)).collect::<Vec<_>>();
        let outcome = check_sequence(
            self.clone_executor_with_symbolic_storage(storage)?,
            &txes,
            &sequence,
            invariant_contract.address,
            invariant_contract.anchor().selector().to_vec().into(),
            CheckSequenceOptions {
                accumulate_warp_roll: config.has_delay(),
                fail_on_revert: config.fail_on_revert,
                expect_assertion_failure,
                call_after_invariant: invariant_contract.call_after_invariant,
                rd: Some(self.revert_decoder()),
            },
        )?;
        Ok((txes, outcome))
    }

    /// Replays persisted handler-side assertion bugs. A file is kept only if the anchor still
    /// asserts at the same `(reverter, selector)` site; stale files (anchor no longer asserts,
    /// asserts at a different site, or earlier call asserts) are deleted in place.
    fn replay_persisted_handler_failures(
        &self,
        handlers_dir: &Path,
        current_settings: &InvariantSettings,
    ) -> (HandlerFailureMap, SymbolicHandlerStorageMap) {
        let mut replayed = HandlerFailureMap::new();
        let mut replayed_storage = SymbolicHandlerStorageMap::default();
        let entries = match std::fs::read_dir(handlers_dir) {
            Ok(entries) => entries,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    error!(%err, "Failed to read handler failure dir");
                }
                return (replayed, replayed_storage);
            }
        };
        let config = &self.config.invariant;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(InvariantPersistedFailure {
                mut call_sequence, storage, failure_site, ..
            }) = persisted_call_sequence(&path, current_settings)
            else {
                continue;
            };
            if call_sequence.is_empty() {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let txes = base_counterexamples_to_txes(&mut call_sequence, config.show_solidity);
            let sequence = (0..min(txes.len(), config.depth as usize)).collect::<Vec<_>>();
            let replay_executor = match self.clone_executor_with_symbolic_storage(&storage) {
                Ok(executor) => executor,
                Err(err) => {
                    error!(%err, "Failed to apply symbolic storage for handler-side assertion replay");
                    continue;
                }
            };
            match replay_handler_failure_sequence(
                replay_executor,
                &txes,
                &sequence,
                config.has_delay(),
                Some(self.revert_decoder()),
            ) {
                Ok(outcome) if outcome.anchor_asserted => {
                    let _ = sh_warn!(
                        "Replayed handler-side assertion bug from {path:?}. \nRun `forge clean` or remove file to ignore."
                    );
                    let actual_site = SymbolicInvariantFailureSite::SequenceCall {
                        target: outcome.reverter,
                        selector: outcome.selector,
                        fingerprint: outcome.anchor_fingerprint,
                    };
                    if failure_site.is_some_and(|expected| expected != actual_site) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    let failure = HandlerAssertionFailure::from_replayed_sequence(
                        txes,
                        outcome.reverter,
                        outcome.selector,
                        outcome.anchor_fingerprint,
                        outcome.revert_reason.unwrap_or_default(),
                    );
                    let site = (failure.reverter, failure.selector);
                    // On collision keep the shorter reproducer.
                    let already_shorter = replayed
                        .get(&site)
                        .and_then(InvariantFuzzError::as_handler_assertion)
                        .is_some_and(|existing| {
                            existing.call_sequence.len() <= failure.call_sequence.len()
                        });
                    if !already_shorter {
                        replayed_storage.insert(
                            (failure.reverter, failure.selector, failure.edge_fingerprint),
                            SymbolicHandlerReplayStorage {
                                call_sequence: failure.call_sequence.clone(),
                                assignments: storage,
                            },
                        );
                        replayed.insert(site, InvariantFuzzError::HandlerAssertion(failure));
                    }
                }
                // Stale: anchor doesn't assert or earlier call asserts.
                Ok(_) => {
                    let _ = std::fs::remove_file(&path);
                }
                Err(err) => {
                    error!(%err, "Failed to replay handler-side assertion bug");
                }
            }
        }
        (replayed, replayed_storage)
    }

    /// Configures this runner with the inline configuration for the contract.
    fn apply_function_inline_config(&mut self, func: &Function) -> Result<()> {
        if self.inline_config.contains_function(self.cr.name, &func.name) {
            let new_config = Arc::new(self.cr.inline_config(Some(func))?);
            self.tcfg.to_mut().reconfigure_with(new_config);
            self.tcfg.configure_executor(self.executor.to_mut());
        }
        Ok(())
    }

    fn run(
        mut self,
        func: &Function,
        invariants: &[&Function],
        kind: TestFunctionKind,
        call_after_invariant: bool,
        identified_contracts: Option<&ContractsByAddress>,
    ) -> TestResult {
        if let Err(e) = self.apply_function_inline_config(func) {
            self.result.single_fail(Some(e.to_string()));
            return self.result;
        }
        let kind = effective_test_function_kind(kind, &self.config, func);

        // In showmap replay mode and `forge fuzz`, only fuzz/invariant tests are runnable.
        if (self.cr.mcr.tcfg.showmap.is_some() || self.cr.mcr.tcfg.fuzz_only)
            && matches!(
                kind,
                TestFunctionKind::UnitTest { .. }
                    | TestFunctionKind::TableTest
                    | TestFunctionKind::SymbolicTest
            )
        {
            if let Some(showmap) = self.cr.mcr.tcfg.showmap.as_ref() {
                let mode = if showmap.emit_files { "showmap" } else { "replay" };
                self.result.replay_skip(format!("not runnable in {mode} mode"));
            } else if self.cr.mcr.tcfg.fuzz_failure_replay {
                self.result
                    .single_skip(SkipReason(Some("not runnable in replay mode".to_string())));
            } else {
                self.result.single_skip(SkipReason(Some("not runnable in fuzz mode".to_string())));
            }
            return self.result;
        }

        match kind {
            TestFunctionKind::UnitTest { .. } => self.run_unit_test(func),
            TestFunctionKind::FuzzTest { .. } => self.run_fuzz_test(func),
            TestFunctionKind::TableTest => self.run_table_test(func),
            TestFunctionKind::SymbolicTest => self.run_symbolic_test(func),
            TestFunctionKind::InvariantTest => {
                let fail_on_revert_for = |f: &Function| {
                    if self.inline_config.contains_function(self.cr.name, &f.name)
                        && let Ok(config) = self.cr.inline_config(Some(f))
                    {
                        return config.invariant.fail_on_revert;
                    }
                    self.config.invariant.fail_on_revert
                };
                let invariant_fns: Vec<_> =
                    invariants.iter().copied().map(|f| (f, fail_on_revert_for(f))).collect();
                self.run_invariant_test(
                    func,
                    invariant_fns,
                    call_after_invariant,
                    identified_contracts.unwrap(),
                )
            }
            _ => unreachable!(),
        }
    }

    /// Runs a single unit test.
    ///
    /// Applies before test txes (if any), runs current test and returns the `TestResult`.
    ///
    /// Before test txes are applied in order and state modifications committed to the EVM database
    /// (therefore the unit test call will be made on modified state).
    /// State modifications of before test txes and unit test function call are discarded after
    /// test ends, similar to `eth_call`.
    fn run_unit_test(mut self, func: &Function) -> TestResult {
        // Prepare unit test execution.
        if self.prepare_test(func).is_err() {
            return self.result;
        }

        // Run current unit test.
        let Ok((mut raw_call_result, reason)) = self.call_test(func, &[]) else {
            return self.result;
        };
        let success =
            self.executor.is_raw_call_mut_success(self.address, &mut raw_call_result, false);
        self.result.single_result(success, reason, raw_call_result);
        self.result
    }

    /// Calls `func` on the test contract, returning the raw result and revert reason. Skipped
    /// and failed calls are recorded in the test result and returned as `Err`.
    fn call_test(
        &mut self,
        func: &Function,
        args: &[DynSolValue],
    ) -> Result<(RawCallResult<FEN>, Option<String>), ()> {
        match self.executor.call(
            self.sender,
            self.address,
            func,
            args,
            U256::ZERO,
            Some(self.revert_decoder()),
        ) {
            Ok(res) => Ok((res.raw, None)),
            Err(EvmError::Execution(err)) => Ok((err.raw, Some(err.reason))),
            Err(EvmError::Skip(reason)) => {
                self.result.single_skip(reason);
                Err(())
            }
            Err(err) => {
                self.result.single_fail(Some(err.to_string()));
                Err(())
            }
        }
    }

    /// Builds the symbolic executor input for one run of `func`.
    fn symbolic_run_input<'f>(
        &'f self,
        func: &'f Function,
        sender: Address,
        collect_success_input: bool,
        corpus_seeds: Vec<SymbolicConcreteInput>,
        branch_target: Option<SymbolicBranchTarget>,
    ) -> SymbolicRunInput<'f, FEN> {
        SymbolicRunInput {
            executor: self.executor.as_ref(),
            target: self.address,
            sender,
            function: func,
            value: U256::ZERO,
            ffi_enabled: self.config.ffi,
            collect_success_input,
            corpus_seeds,
            branch_target,
        }
    }

    /// Sets the per-test corpus directory in `fuzz_config` and returns the test path name, the
    /// legacy corpus directory and the persisted failure `(dir, file)` paths.
    fn fuzz_test_paths<'f>(
        &self,
        func: &'f Function,
        fuzz_config: &mut FuzzConfig,
    ) -> (Cow<'f, str>, Option<PathBuf>, (PathBuf, PathBuf)) {
        let test_name = fuzz_test_path_name(&self.cr.contract.abi, func, fuzz_config, self.cr.name);
        let legacy_corpus_dir = legacy_fuzz_corpus_dir(
            fuzz_config.corpus.corpus_dir.as_deref(),
            self.cr.name,
            func,
            &test_name,
        );
        let failure_paths = test_paths(
            &mut fuzz_config.corpus,
            fuzz_config.failure_persist_dir.clone().unwrap(),
            self.cr.name,
            &test_name,
        );
        (test_name, legacy_corpus_dir, failure_paths)
    }

    /// Imports persisted fuzz corpus entries as symbolic path-priority hints.
    fn import_symbolic_fuzz_corpus(
        &self,
        func: &Function,
    ) -> (Vec<SymbolicConcreteInput>, Option<SymbolicCorpusSeedMetadata>) {
        let mut inputs = Vec::new();
        if !should_symbolically_import_fuzz_corpus(&self.config, func) {
            return (inputs, None);
        }

        let mut fuzz_config = self.config.fuzz.clone();
        let (_, legacy_corpus_dir, _) = self.fuzz_test_paths(func, &mut fuzz_config);
        let corpus_dir = legacy_corpus_dir.or(fuzz_config.corpus.corpus_dir);
        let limit = self.config.symbolic.corpus_seed_limit;
        let mut metadata = SymbolicCorpusSeedMetadata {
            corpus_dir: corpus_dir.clone(),
            limit,
            loaded: 0,
            skipped: 0,
            used: Vec::new(),
        };
        let Some(corpus_dir) = corpus_dir else {
            let _ = sh_warn!(
                "`--symbolic-use-fuzz-corpus` requires `--fuzz-corpus-dir` or `fuzz.corpus_dir`; \
                 running without imported corpus seeds"
            );
            return (inputs, Some(metadata));
        };
        if limit == 0 {
            return (inputs, Some(metadata));
        }

        'dirs: for replay_dir in canonical_replay_dirs(&corpus_dir) {
            let mut entries = read_corpus_dir(&replay_dir).collect::<Vec<_>>();
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            for entry in entries {
                if inputs.len() >= limit {
                    break 'dirs;
                }
                metadata.loaded += 1;
                let input = match entry.read_tx_seq() {
                    Ok(tx_seq) => self.symbolic_corpus_seed_input(func, &tx_seq),
                    Err(err) => {
                        debug!(%err, path = %entry.path.display(), "failed to read symbolic corpus seed");
                        None
                    }
                };
                let Some(input) = input else {
                    metadata.skipped += 1;
                    continue;
                };
                metadata.used.push(SymbolicCorpusSeedRef {
                    path: entry.path,
                    calldata: input.calldata.clone(),
                });
                inputs.push(input);
            }
        }

        debug!(
            test = %func.signature(),
            corpus_dir = %corpus_dir.display(),
            loaded = metadata.loaded,
            skipped = metadata.skipped,
            imported = inputs.len(),
            "imported symbolic fuzz corpus seeds"
        );
        (inputs, Some(metadata))
    }

    /// Imports persisted fuzz branch frontiers as `(id, sender, branch target, input)` seeds.
    fn import_symbolic_fuzz_frontiers(
        &self,
        func: &Function,
        fuzz_config: &FuzzConfig,
    ) -> Vec<(u64, Address, SymbolicBranchTarget, SymbolicConcreteInput)> {
        let limit = self.config.symbolic.frontier_limit;
        if limit == 0 {
            return Vec::new();
        }

        let Some(frontier_dir) = fuzz_config.corpus.frontier_dir.as_ref() else {
            let _ = sh_warn!(
                "`--symbolic-use-fuzz-frontiers` requires `--fuzz-frontier-dir` or \
                 `fuzz.frontier_dir`; running without targeted frontier seeds"
            );
            return Vec::new();
        };

        let frontier_path = frontier_dir.join(FUZZ_BRANCH_FRONTIER_FILE);
        let artifact = match foundry_common::fs::read_json_file::<FuzzBranchFrontierArtifact>(
            &frontier_path,
        ) {
            Ok(artifact) => artifact,
            Err(err) => {
                debug!(
                    %err,
                    path = %frontier_path.display(),
                    "failed to read fuzz branch frontier artifact"
                );
                return Vec::new();
            }
        };

        if artifact.schema != FUZZ_BRANCH_FRONTIER_SCHEMA || artifact.version != 1 {
            warn!(
                schema = %artifact.schema,
                version = artifact.version,
                path = %frontier_path.display(),
                "unsupported fuzz branch frontier artifact"
            );
            return Vec::new();
        }
        let signature = func.signature();
        if artifact.test != signature {
            warn!(
                artifact_test = %artifact.test,
                test = %signature,
                path = %frontier_path.display(),
                "fuzz branch frontier artifact does not match symbolic target"
            );
            return Vec::new();
        }

        let requested_ids = &self.config.symbolic.frontier_ids;
        let requested_pcs = &self.config.symbolic.frontier_pcs;
        let requested_selectors = &self.config.symbolic.frontier_selectors;
        let parsed_selectors = parse_frontier_selectors(requested_selectors, &signature);
        let selection_active = !requested_ids.is_empty()
            || !requested_pcs.is_empty()
            || !requested_selectors.is_empty();
        let mut skipped_by_selection = 0usize;
        let mut imported_ids = Vec::new();
        let mut imported_pcs = Vec::new();
        let mut imported_selectors = Vec::new();
        let mut imported = Vec::with_capacity(limit.min(artifact.frontiers.len()));
        for frontier in artifact.frontiers {
            let selector = frontier_selector(&frontier);
            if (!requested_ids.is_empty() && !requested_ids.contains(&frontier.id))
                || (!requested_pcs.is_empty() && !requested_pcs.contains(&frontier.site.pc))
                || (!requested_selectors.is_empty()
                    && selector.is_none_or(|selector| !parsed_selectors.contains(&selector)))
            {
                skipped_by_selection += 1;
                continue;
            }
            if imported.len() == limit {
                if selection_active {
                    continue;
                }
                break;
            }
            if !matches!(
                frontier.site.opcode,
                opcode::EQ | opcode::LT | opcode::GT | opcode::SLT | opcode::SGT | opcode::ISZERO
            ) {
                debug!(
                    opcode = frontier.site.opcode,
                    id = frontier.id,
                    "skipping unsupported fuzz branch frontier opcode"
                );
                continue;
            }
            let ([call], 0) = (frontier.sequence.as_slice(), frontier.call_index) else {
                debug!(
                    id = frontier.id,
                    sequence_len = frontier.sequence.len(),
                    call_index = frontier.call_index,
                    "skipping non-stateless fuzz branch frontier"
                );
                continue;
            };
            let Some(input) = self.symbolic_corpus_seed_input(func, std::slice::from_ref(call))
            else {
                debug!(id = frontier.id, "skipping fuzz branch frontier with incompatible call");
                continue;
            };
            let target = SymbolicBranchTarget::new(
                frontier.site.address,
                frontier.site.pc,
                frontier.site.opcode,
                frontier.operands.result,
            );
            imported.push((frontier.id, call.sender, target, input));
            imported_ids.push(frontier.id);
            imported_pcs.push(frontier.site.pc);
            imported_selectors.extend(selector);
        }

        warn_unimported_frontiers("id", requested_ids, &imported_ids, &signature, &frontier_path);
        warn_unimported_frontiers("pc", requested_pcs, &imported_pcs, &signature, &frontier_path);
        warn_unimported_frontiers(
            "selector",
            &parsed_selectors,
            &imported_selectors,
            &signature,
            &frontier_path,
        );
        if selection_active {
            let _ = sh_status!(
                "Symbolic frontier selection for {signature}: imported {}, skipped {} by target \
                 filters (ids: {}; pcs: {}; selectors: {}; limit: {limit})",
                imported.len(),
                skipped_by_selection,
                frontier_filter_display(requested_ids),
                frontier_filter_display(requested_pcs),
                frontier_filter_display(requested_selectors),
            );
        }

        debug!(
            test = %signature,
            path = %frontier_path.display(),
            imported = imported.len(),
            limit,
            skipped_by_selection,
            requested_ids = ?requested_ids,
            requested_pcs = ?requested_pcs,
            requested_selectors = ?requested_selectors,
            "imported fuzz branch frontiers for targeted symbolic seeding"
        );
        imported
    }

    fn symbolic_corpus_seed_input(
        &self,
        func: &Function,
        tx_seq: &[BasicTxDetails],
    ) -> Option<SymbolicConcreteInput> {
        let [tx] = tx_seq else {
            return None;
        };
        if tx.call_details.target != self.address
            || !tx.call_details.value.unwrap_or_default().is_zero()
        {
            return None;
        }
        let calldata = &tx.call_details.calldata;
        if calldata.get(..4) != Some(func.selector().as_slice()) {
            return None;
        }
        let args = func.abi_decode_input(&calldata[4..]).ok()?;
        Some(SymbolicConcreteInput { args, calldata: calldata.clone() })
    }

    /// Runs a symbolic test and replays any discovered counterexample concretely.
    fn run_symbolic_test(mut self, func: &Function) -> TestResult {
        if self.prepare_test(func).is_err() {
            return self.result;
        }

        let (corpus_seeds, mut corpus_seed_metadata) = self.import_symbolic_fuzz_corpus(func);
        if let Some(metadata) = corpus_seed_metadata.as_mut() {
            match SymbolicExecutor::modeled_corpus_seed_indexes(
                &self.config.symbolic,
                func,
                &corpus_seeds,
            ) {
                Ok(indexes) => {
                    let mut indexes = indexes.into_iter().peekable();
                    metadata.used = std::mem::take(&mut metadata.used)
                        .into_iter()
                        .enumerate()
                        .filter_map(|(idx, seed)| indexes.next_if_eq(&idx).map(|_| seed))
                        .collect();
                }
                Err(err) => {
                    debug!(
                        %err,
                        test = %func.signature(),
                        "failed to model imported symbolic corpus seeds"
                    );
                }
            }
        }
        let symbolic_config = self.config.symbolic.clone();
        let mut symbolic = SymbolicExecutor::new(symbolic_config.clone());
        // Progress rendering must finish before verbose SMT diagnostics are printed.
        if self.cr.progress.is_some() && symbolic_config.dump_smt {
            symbolic.capture_diagnostics();
        }
        let result =
            symbolic.run(self.symbolic_run_input(func, self.sender, false, corpus_seeds, None));
        let portfolio_diagnostics = symbolic.portfolio_diagnostics();
        let symbolic_diagnostics = symbolic.take_diagnostics();

        let (status, reason, counterexample, symbolic_result) = match result {
            SymbolicRunResult::Safe { stats, .. } => {
                (TestStatus::Success, None, None, SymbolicResult::pass(&symbolic_config, stats))
            }
            SymbolicRunResult::Incomplete { kind, reason, stats } => (
                TestStatus::Failure,
                Some(format!("incomplete symbolic execution ({kind:?}): {reason}")),
                None,
                SymbolicResult::incomplete(
                    &symbolic_config,
                    kind,
                    reason,
                    stats,
                    SymbolicReplayMetadata::not_required(),
                    SymbolicCallTrace::none(),
                    None,
                ),
            ),
            SymbolicRunResult::Counterexample { args, calldata, stats } => {
                self.replay_symbolic_counterexample(func, args, calldata, stats, &symbolic_config)
            }
        };
        let symbolic_result = match corpus_seed_metadata {
            Some(metadata) => symbolic_result.with_corpus_seeds(metadata),
            None => symbolic_result,
        };
        self.result.symbolic_result(status, reason, counterexample, symbolic_result);
        self.result.symbolic_portfolio_diagnostics = portfolio_diagnostics;
        self.result.symbolic_diagnostics = symbolic_diagnostics;
        self.result
    }

    /// Replays a symbolic counterexample concretely, minimizing and persisting it when it
    /// reproduces.
    fn replay_symbolic_counterexample(
        &mut self,
        func: &Function,
        args: Vec<DynSolValue>,
        calldata: Bytes,
        stats: SymbolicStats,
        symbolic_config: &SymbolicConfig,
    ) -> (TestStatus, Option<String>, Option<CounterExample>, SymbolicResult) {
        let symbolic_counterexample = SymbolicCounterexample::from(
            &BaseCounterExample::from_fuzz_call(calldata.clone(), args.clone(), None),
        );
        let incomplete = |reason: String, replay, call_trace| {
            SymbolicResult::incomplete(
                symbolic_config,
                SymbolicStopReason::Error,
                reason,
                stats,
                replay,
                call_trace,
                Some(symbolic_counterexample.clone()),
            )
        };

        let (raw, reason) = match self.executor.call(
            self.sender,
            self.address,
            func,
            &args,
            U256::ZERO,
            Some(self.revert_decoder()),
        ) {
            Ok(res) => (res.raw, None),
            Err(EvmError::Execution(err)) => (err.raw, Some(err.reason)),
            Err(EvmError::Skip(reason)) => {
                let replay_reason = format!("vm.skip during concrete replay: {reason}");
                let symbolic_result = incomplete(
                    "concrete replay skipped the symbolic counterexample".to_string(),
                    SymbolicReplayMetadata::skipped(replay_reason),
                    SymbolicCallTrace::none(),
                );
                return (TestStatus::Skipped, reason.0, None, symbolic_result);
            }
            Err(err) => {
                let reason = err.to_string();
                let symbolic_result = incomplete(
                    reason.clone(),
                    SymbolicReplayMetadata::error(reason.clone()),
                    SymbolicCallTrace::none(),
                );
                return (TestStatus::Failure, Some(reason), None, symbolic_result);
            }
        };

        let base_counterexample =
            BaseCounterExample::from_fuzz_call(calldata, args, raw.traces.clone());
        if self.executor.is_raw_call_success(
            self.address,
            Cow::Borrowed(&raw.state_changeset),
            &raw,
            false,
        ) {
            // The solver model is not a user-facing counterexample until replay confirms it, so
            // report the mismatch as an incomplete run instead.
            let call_trace = SymbolicCallTrace::test_result_traces(raw.traces.is_some());
            self.result.extend(raw);
            let reason = "symbolic counterexample did not replay".to_string();
            let display_reason = format!(
                "incomplete symbolic execution ({:?}): {reason}",
                SymbolicStopReason::Error
            );
            let symbolic_result =
                incomplete(reason.clone(), SymbolicReplayMetadata::mismatch(reason), call_trace);
            return (TestStatus::Failure, Some(display_reason), None, symbolic_result);
        }

        let original_call = SymbolicCounterexampleCall::from_base_counterexample(
            &base_counterexample,
            self.sender,
            self.address,
        );
        let mut final_call = original_call.clone();
        let mut final_raw = raw;
        let mut final_reason = reason;
        let mut minimization = None;
        if final_reason.is_some()
            && let Some(candidate) = minimize_single_call_counterexample(
                func,
                &original_call,
                self.tcfg.config.invariant.shrink_run_limit as usize,
                |candidate| {
                    self.replay_confirmed_symbolic_single_call(candidate, final_reason.as_deref())
                        .is_ok()
                },
            )
        {
            if candidate.changed() {
                match self.replay_confirmed_symbolic_single_call(
                    &candidate.minimized_call,
                    final_reason.as_deref(),
                ) {
                    Ok((raw, reason)) => {
                        final_call = candidate.minimized_call.clone();
                        final_raw = raw;
                        final_reason = reason;
                        minimization = Some(candidate);
                    }
                    Err(err) => {
                        warn!(
                            %err,
                            "discarding symbolic counterexample minimization result that no longer replays"
                        );
                    }
                }
            } else {
                minimization = Some(candidate);
            }
        }

        let call_trace = SymbolicCallTrace::test_result_traces(final_raw.traces.is_some());
        let mut base_counterexample = final_call.to_base_counterexample();
        base_counterexample.traces = final_raw.traces.clone();
        self.result.extend(final_raw);

        let signature = func.signature();
        let fail_on_revert = self.config.invariant.fail_on_revert;
        let kind = SymbolicCounterexampleArtifactKind::SingleCall;
        let mut symbolic_result = SymbolicResult::fail_counterexample(
            symbolic_config,
            stats,
            call_trace,
            SymbolicCounterexample::from(&base_counterexample),
        );
        let minimized_artifact = self.write_symbolic_artifact(
            &signature,
            &self.symbolic_artifact(
                &signature,
                kind,
                &symbolic_result,
                fail_on_revert,
                vec![final_call],
            ),
        );
        if let Some(artifact) = minimized_artifact.clone() {
            symbolic_result = symbolic_result.with_artifact(artifact);
        }
        if let Some(minimization) = minimization {
            let original_result = SymbolicResult::fail_counterexample(
                symbolic_config,
                stats,
                SymbolicCallTrace::none(),
                symbolic_counterexample,
            );
            let original_artifact = self.write_symbolic_artifact(
                &format!("original__{signature}"),
                &self.symbolic_artifact(
                    &signature,
                    kind,
                    &original_result,
                    fail_on_revert,
                    vec![minimization.original_call.clone()],
                ),
            );
            if let Some((original, minimized)) = original_artifact.zip(minimized_artifact) {
                symbolic_result =
                    symbolic_result.with_minimization(SymbolicCounterexampleMinimization::new(
                        original,
                        minimized,
                        minimization.attempts,
                        minimization.accepted,
                        minimization.original_call.calldata.len(),
                        minimization.minimized_call.calldata.len(),
                    ));
            }
        }
        (
            TestStatus::Failure,
            final_reason,
            Some(CounterExample::Single(base_counterexample)),
            symbolic_result,
        )
    }

    /// Replays a durable symbolic counterexample artifact against this freshly set up test.
    fn run_symbolic_artifact_replay(
        mut self,
        func: &Function,
        invariants: &[&Function],
        call_after_invariant: bool,
    ) -> TestResult {
        if let Err(reason) = self.replay_symbolic_artifact(func, invariants, call_after_invariant) {
            self.result.single_fail(Some(reason));
        }
        self.result
    }

    /// Replays a persisted symbolic counterexample artifact against `func`, failing with the
    /// mismatch reason when the recorded outcome does not reproduce.
    fn replay_symbolic_artifact(
        &mut self,
        func: &Function,
        invariants: &[&Function],
        call_after_invariant: bool,
    ) -> Result<(), String> {
        let Some(replay) = &self.cr.mcr.tcfg.symbolic_artifact_replay else {
            return Err("missing symbolic artifact replay config".to_string());
        };
        let artifact = &replay.artifact;
        self.apply_function_inline_config(func).map_err(|e| e.to_string())?;

        match artifact.kind {
            SymbolicCounterexampleArtifactKind::SingleCall => {
                if artifact.replay.status != SymbolicReplayStatus::Confirmed {
                    return Err(format!(
                        "single-call symbolic artifact replay status must be confirmed, got {:?}",
                        artifact.replay.status
                    ));
                }
                let Some(call) = artifact.calls.first() else {
                    return Err("symbolic artifact has no calls".to_string());
                };
                if artifact.calls.len() != 1 {
                    return Err(
                        "single-call symbolic artifact must contain exactly one call".to_string()
                    );
                }
                // Single-call artifacts are concrete replay inputs: sender, value, warp, and roll
                // are intentionally taken from the artifact. Validation only checks that the call
                // still targets this test function.
                if call.target != self.address {
                    return Err(format!(
                        "single-call symbolic artifact target {} does not match test contract {}",
                        call.target, self.address
                    ));
                }
                if call.calldata.get(..4).is_none_or(|selector| func.selector() != selector) {
                    return Err(format!(
                        "single-call symbolic artifact calldata does not match `{}` selector",
                        func.signature()
                    ));
                }

                if self.prepare_test(func).is_err() {
                    return Ok(());
                }

                let counterexample = || CounterExample::Single(call.to_base_counterexample());
                let mut executor = self.clone_executor();
                let raw = match execute_tx(&mut executor, &call.to_basic_tx_details()) {
                    Ok(raw) => raw,
                    Err(err) => {
                        self.result.counterexample = Some(counterexample());
                        return Err(err.to_string());
                    }
                };
                if executor.is_raw_call_success(
                    self.address,
                    Cow::Borrowed(&raw.state_changeset),
                    &raw,
                    false,
                ) {
                    self.result.single_result(true, None, raw);
                    return Ok(());
                }
                match raw.into_evm_error(Some(self.revert_decoder())) {
                    EvmError::Execution(err) => {
                        let reason = if err.reason.is_empty() {
                            artifact.replay.reason.clone()
                        } else {
                            Some(err.reason.clone())
                        };
                        self.result.single_result(false, reason, err.raw);
                        self.result.counterexample = Some(counterexample());
                    }
                    EvmError::Skip(reason) => self.result.single_skip(reason),
                    err => {
                        self.result.counterexample = Some(counterexample());
                        return Err(err.to_string());
                    }
                }
            }
            SymbolicCounterexampleArtifactKind::Sequence => {
                let Some(invariant) = invariants.first() else {
                    return Err(
                        "sequence symbolic artifact must target an invariant test".to_string()
                    );
                };
                if artifact.calls.is_empty() {
                    return Err("symbolic artifact has no calls".to_string());
                }

                let calls = artifact
                    .calls
                    .iter()
                    .map(SymbolicCounterexampleCall::to_base_counterexample)
                    .collect::<Vec<_>>();
                let txes = artifact
                    .calls
                    .iter()
                    .map(SymbolicCounterexampleCall::to_basic_tx_details)
                    .collect::<Vec<_>>();
                let setup_contracts = load_contracts(
                    self.setup.traces.iter().map(|(_, trace)| &trace.arena),
                    &self.cr.mcr.known_contracts,
                );
                let mut evm = InvariantExecutor::new_with_fuzz_seed(
                    self.clone_executor(),
                    self.invariant_runner(),
                    self.config.fuzz.seed,
                    self.config.invariant.clone(),
                    &setup_contracts,
                    &self.cr.mcr.known_contracts,
                    self.cr.num_invariant_campaign_anchors,
                );
                if let Err(err) = evm.select_contract_artifacts(self.address) {
                    self.result.invariant_setup_fail(err);
                    return Ok(());
                }
                let (sender_filters, targeted) =
                    match evm.select_contracts_and_senders(self.address) {
                        Ok(selected) => selected,
                        Err(err) => {
                            self.result.invariant_setup_fail(err);
                            return Ok(());
                        }
                    };
                let artifact_executor =
                    match self.clone_executor_with_symbolic_storage(&artifact.storage) {
                        Ok(executor) => executor,
                        Err(err) => {
                            self.result.counterexample =
                                Some(CounterExample::Sequence(calls.len(), calls));
                            return Err(err.to_string());
                        }
                    };

                let dynamic_target_ctx = evm.dynamic_target_ctx();
                let mut validation_executor =
                    targeted.is_updatable.then(|| artifact_executor.clone());
                let mut validation_created_contracts = Vec::new();
                for (idx, tx) in txes.iter().enumerate() {
                    let Some(selector) = tx.call_details.calldata.get(..4) else {
                        return Err(format!(
                            "sequence symbolic artifact call {} has calldata shorter than a selector",
                            idx + 1
                        ));
                    };
                    if !targeted.targets().can_replay(tx) {
                        return Err(format!(
                            "sequence symbolic artifact call {} targets unknown function {} on {}",
                            idx + 1,
                            hex::encode_prefixed(selector),
                            tx.call_details.target
                        ));
                    }
                    if (!sender_filters.targeted.is_empty()
                        && !sender_filters.targeted.contains(&tx.sender))
                        || sender_filters.excluded.contains(&tx.sender)
                    {
                        return Err(format!(
                            "sequence symbolic artifact call {} uses forbidden sender {}",
                            idx + 1,
                            tx.sender
                        ));
                    }
                    if let Some(validation_executor) = validation_executor.as_mut() {
                        execute_tx_and_register_created(
                            validation_executor,
                            tx,
                            &targeted,
                            &dynamic_target_ctx,
                            &mut validation_created_contracts,
                        )
                        .map_err(|err| {
                            format!(
                                "sequence symbolic artifact call {} failed during target validation: {err}",
                                idx + 1
                            )
                        })?;
                    }
                }

                let artifact_failure = artifact.invariant_failure.as_ref();
                if matches!(
                    artifact_failure,
                    Some(SymbolicInvariantArtifactFailure::Predicate { site: None, .. })
                ) {
                    return Err(
                        "sequence symbolic artifact does not identify an exact predicate failure site"
                            .to_string(),
                    );
                }
                let is_handler_artifact = matches!(
                    artifact_failure,
                    Some(SymbolicInvariantArtifactFailure::Handler { .. })
                );
                let sequence = (0..txes.len()).collect::<Vec<_>>();
                let outcome = match check_sequence(
                    artifact_executor,
                    &txes,
                    &sequence,
                    self.setup.address,
                    invariant.selector().to_vec().into(),
                    CheckSequenceOptions {
                        // Artifact replay executes every stored call in order, so each call's
                        // warp/roll delta is applied directly. Accumulation is only needed when a
                        // shrink candidate skips calls and must fold removed delays forward.
                        accumulate_warp_roll: false,
                        fail_on_revert: is_handler_artifact
                            || artifact.replay_semantics.fail_on_revert,
                        expect_assertion_failure: is_handler_artifact,
                        call_after_invariant,
                        rd: Some(self.revert_decoder()),
                    },
                ) {
                    Ok(outcome) => outcome,
                    Err(err) => {
                        self.result.counterexample =
                            Some(CounterExample::Sequence(calls.len(), calls));
                        return Err(err.to_string());
                    }
                };
                if outcome.success {
                    self.result.invariant_replay_success(outcome.calls_count, outcome.reverts);
                    return Ok(());
                }
                match artifact_failure {
                    Some(SymbolicInvariantArtifactFailure::Handler {
                        name,
                        reverter,
                        selector,
                        fingerprint,
                    }) => {
                        let expected_site = CheckSequenceFailureSite::SequenceCall {
                            target: *reverter,
                            selector: *selector,
                            fingerprint: *fingerprint,
                        };
                        if outcome.failure_site != Some(expected_site) {
                            return Err(format!(
                                "sequence symbolic artifact replayed a different handler \
                                 failure site than the stored artifact: expected \
                                 {reverter}::{selector} at {fingerprint}, got {:?}",
                                outcome.failure_site
                            ));
                        }
                        let handler_name = name.clone().unwrap_or_else(|| {
                            invariant_handler_failure_name(&setup_contracts, *reverter, *selector)
                        });
                        self.result.invariant_result(
                            invariant_kind(1, outcome.calls_count, outcome.reverts),
                            InvariantOutcome {
                                handler_failures: vec![InvariantFailure::Handler {
                                    name: handler_name,
                                    reverter: *reverter,
                                    selector: *selector,
                                    reason: outcome
                                        .reason
                                        .or_else(|| artifact.replay.reason.clone())
                                        .unwrap_or_else(|| {
                                            "symbolic handler counterexample".to_string()
                                        }),
                                    counterexample: Some(CounterExample::Sequence(
                                        calls.len(),
                                        calls,
                                    )),
                                    artifact: Some(SymbolicArtifactRef::new(replay.path.clone())),
                                }],
                                ..Default::default()
                            },
                        );
                    }
                    _ => {
                        if let Some(SymbolicInvariantArtifactFailure::Predicate { site, .. }) =
                            artifact_failure
                            && outcome.failure_site.map(SymbolicInvariantFailureSite::from) != *site
                        {
                            return Err(format!(
                                "sequence symbolic artifact replayed a different failure \
                                 origin than the stored predicate: got {:?}",
                                outcome.failure_site
                            ));
                        }
                        let signature = invariant.signature();
                        let invariant_name = match artifact_failure {
                            Some(SymbolicInvariantArtifactFailure::Predicate { name, .. }) => {
                                name.as_str()
                            }
                            _ => signature.as_str(),
                        };
                        self.result.invariant_replay_fail(
                            outcome,
                            invariant_name,
                            artifact.replay.reason.clone(),
                            calls,
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn try_seed_fuzz_corpus_from_frontiers(&self, func: &Function, fuzz_config: &FuzzConfig) {
        if !self.config.symbolic.use_fuzz_frontiers || !func.test_function_kind().is_fuzz_test() {
            return;
        }
        if fuzz_config.corpus.corpus_dir.is_none() {
            let _ = sh_warn!(
                "`--symbolic-use-fuzz-frontiers` requires `--fuzz-corpus-dir` or \
                 `fuzz.corpus_dir`; skipping targeted frontier seeding"
            );
            return;
        }

        for (id, sender, target, input) in self.import_symbolic_fuzz_frontiers(func, fuzz_config) {
            let mut symbolic = SymbolicExecutor::new(self.config.symbolic.clone());
            let result = symbolic.run(self.symbolic_run_input(
                func,
                sender,
                true,
                vec![input],
                Some(target),
            ));

            let (input, expect_failure) = match result {
                SymbolicRunResult::Safe { success_input: Some(input), .. } => (input, false),
                SymbolicRunResult::Safe { success_input: None, .. } => {
                    warn!(
                        id,
                        test = %func.signature(),
                        "targeted symbolic frontier produced no branch-flipping input"
                    );
                    continue;
                }
                SymbolicRunResult::Incomplete { kind, reason, .. } => {
                    warn!(
                        id,
                        ?kind,
                        %reason,
                        test = %func.signature(),
                        "targeted symbolic frontier incomplete"
                    );
                    continue;
                }
                SymbolicRunResult::Counterexample { args, calldata, .. } => {
                    (SymbolicConcreteInput { args, calldata }, true)
                }
            };

            let replay = self.symbolic_fuzz_seed_replay(sender, &input, fuzz_config);
            if replay != Some(!expect_failure) {
                warn!(
                    id,
                    ?replay,
                    test = %func.signature(),
                    "targeted symbolic frontier seed did not replay with the expected outcome"
                );
                continue;
            }

            match self.persist_symbolic_fuzz_seed(&fuzz_config.corpus, sender, input.calldata) {
                Ok(Some(path)) => {
                    debug!(
                        id,
                        path = %path.display(),
                        test = %func.signature(),
                        "persisted targeted symbolic frontier seed"
                    );
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        %err,
                        id,
                        test = %func.signature(),
                        "failed to persist targeted symbolic frontier seed"
                    );
                }
            }
        }
    }

    fn try_seed_fuzz_corpus_symbolically(&self, func: &Function, fuzz_config: &FuzzConfig) {
        if !self.config.symbolic.seed_corpus || !func.test_function_kind().is_fuzz_test() {
            return;
        }
        if fuzz_config.corpus.corpus_dir.is_none() {
            let _ = sh_warn!(
                "`--symbolic-seed-corpus` requires `--fuzz-corpus-dir` or `fuzz.corpus_dir`; \
                 skipping symbolic corpus seeding"
            );
            return;
        }

        let mut symbolic = SymbolicExecutor::new(self.config.symbolic.clone());
        let result =
            symbolic.run(self.symbolic_run_input(func, self.sender, true, Vec::new(), None));

        let input = match result {
            SymbolicRunResult::Safe { success_input: Some(input), .. } => input,
            SymbolicRunResult::Safe { success_input: None, .. } => {
                warn!(test = %func.signature(), "symbolic fuzz corpus seeding found no successful input");
                return;
            }
            SymbolicRunResult::Incomplete { kind, reason, .. } => {
                warn!(?kind, %reason, test = %func.signature(), "symbolic fuzz corpus seeding incomplete");
                return;
            }
            SymbolicRunResult::Counterexample { .. } => {
                warn!(test = %func.signature(), "symbolic fuzz corpus seeding found a counterexample");
                return;
            }
        };

        if self.symbolic_fuzz_seed_replay(self.sender, &input, fuzz_config) != Some(true) {
            warn!(test = %func.signature(), "symbolic fuzz corpus seed did not pass concrete replay");
            return;
        }

        if let Err(err) =
            self.persist_symbolic_fuzz_seed(&fuzz_config.corpus, self.sender, input.calldata)
        {
            warn!(%err, test = %func.signature(), "failed to persist symbolic fuzz corpus seed");
        }
    }

    /// Persists a concretely confirmed symbolic input as a fuzz corpus seed.
    fn persist_symbolic_fuzz_seed(
        &self,
        corpus: &FuzzCorpusConfig,
        sender: Address,
        calldata: Bytes,
    ) -> foundry_common::fs::Result<Option<PathBuf>> {
        persist_corpus_seed(
            corpus,
            vec![BasicTxDetails {
                warp: None,
                roll: None,
                sender,
                call_details: CallDetails {
                    target: self.address,
                    calldata,
                    value: Some(U256::ZERO),
                },
            }],
        )
    }

    /// Replays a symbolic seed concretely: `Some(success)`, or `None` if the input was rejected.
    fn symbolic_fuzz_seed_replay(
        &self,
        sender: Address,
        input: &SymbolicConcreteInput,
        fuzz_config: &FuzzConfig,
    ) -> Option<bool> {
        let raw = self
            .clone_executor()
            .call_raw(sender, self.address, input.calldata.clone(), U256::ZERO)
            .ok()?;
        if raw.result.as_ref() == MAGIC_ASSUME {
            return None;
        }
        Some(
            should_ignore_revert(
                fuzz_config.fail_on_revert,
                self.address,
                raw.reverter,
                self.executor.inspector().extra_cheatcode_addresses(),
            ) || self.executor.is_raw_call_success(
                self.address,
                Cow::Borrowed(&raw.state_changeset),
                &raw,
                false,
            ),
        )
    }

    /// Runs a table test.
    /// The parameters dataset (table) is created from defined parameter fixtures, therefore each
    /// test table parameter should have the same number of fixtures defined.
    /// E.g. for table test
    /// - `table_test(uint256 amount, bool swap)` fixtures are defined as
    /// - `uint256[] public fixtureAmount = [2, 5]`
    /// - `bool[] public fixtureSwap = [true, false]` The `table_test` is then called with the pair
    ///   of args `(2, true)` and `(5, false)`.
    fn run_table_test(mut self, func: &Function) -> TestResult {
        // Prepare unit test execution.
        if self.prepare_test(func).is_err() {
            return self.result;
        }

        // Extract and validate fixtures for the first table test parameter.
        let Some(first_param) = func.inputs.first() else {
            self.result.single_fail(Some("Table test should have at least one parameter".into()));
            return self.result;
        };

        let Some(first_param_fixtures) =
            &self.setup.fuzz_fixtures.param_fixtures(first_param.name())
        else {
            self.result.single_fail(Some("Table test should have fixtures defined".into()));
            return self.result;
        };

        if first_param_fixtures.is_empty() {
            self.result.single_fail(Some("Table test should have at least one fixture".into()));
            return self.result;
        }

        let fixtures_len = first_param_fixtures.len();
        let mut table_fixtures = vec![&first_param_fixtures[..]];

        // Collect fixtures for remaining parameters.
        for param in &func.inputs[1..] {
            let param_name = param.name();
            let Some(fixtures) = &self.setup.fuzz_fixtures.param_fixtures(param.name()) else {
                self.result.single_fail(Some(format!("No fixture defined for param {param_name}")));
                return self.result;
            };

            if fixtures.len() != fixtures_len {
                self.result.single_fail(Some(format!(
                    "{} fixtures defined for {param_name} (expected {})",
                    fixtures.len(),
                    fixtures_len
                )));
                return self.result;
            }

            table_fixtures.push(&fixtures[..]);
        }

        let progress = self.fuzz_progress(&func.name, None, fixtures_len as u32);

        let mut result = FuzzTestResult::default();

        for i in 0..fixtures_len {
            if self.tcfg.early_exit.should_stop() {
                return self.result;
            }

            // Increment progress bar.
            if let Some(progress) = progress.as_ref() {
                progress.inc(1);
            }

            let args = table_fixtures.iter().map(|row| row[i].clone()).collect_vec();
            let Ok((mut raw_call_result, reason)) = self.call_test(func, &args) else {
                return self.result;
            };

            result.gas_by_case.push((raw_call_result.gas_used, raw_call_result.stipend));
            result.logs.extend(raw_call_result.logs.clone());
            result.labels.extend(raw_call_result.labels.clone());
            HitMaps::merge_opt(&mut result.line_coverage, raw_call_result.line_coverage.clone());

            let is_success =
                self.executor.is_raw_call_mut_success(self.address, &mut raw_call_result, false);
            // Record counterexample if test fails.
            if !is_success {
                result.counterexample =
                    Some(CounterExample::Single(BaseCounterExample::from_fuzz_call(
                        Bytes::from(func.abi_encode_input(&args).unwrap()),
                        args,
                        raw_call_result.traces.clone(),
                    )));
                result.reason = reason;
            }
            // Stop on the first failure, or after the last row using its call result for logs
            // and traces.
            if !is_success || i == fixtures_len - 1 {
                result.success = is_success;
                result.traces = raw_call_result.traces;
                result.debug_bytecodes = raw_call_result.debug_bytecodes;
                self.result.table_result(result);
                return self.result;
            }
        }

        self.result
    }

    fn run_invariant_test(
        mut self,
        func: &Function,
        invariants: Vec<(&Function, bool)>,
        call_after_invariant: bool,
        identified_contracts: &ContractsByAddress,
    ) -> TestResult {
        let fuzz_failure_replay = self.cr.mcr.tcfg.fuzz_failure_replay;
        let mut invariant_config = self.config.invariant.clone();
        if fuzz_failure_replay {
            invariant_config.runs = 0;
        }
        let invariant_config = &invariant_config;
        let is_optimization = is_optimization_invariant(func);

        let mut live_invariants = Vec::new();
        let mut skipped_predicate_results = Vec::new();
        for (invariant, fail_on_revert) in invariants {
            if let Some(reason) = self.invariant_skip_reason(invariant) {
                skipped_predicate_results.push(InvariantPredicateResult {
                    name: invariant.name.clone(),
                    status: TestStatus::Skipped,
                    reason: reason.0,
                });
            } else {
                live_invariants.push((invariant, fail_on_revert));
            }
        }

        if live_invariants.is_empty() {
            let skip_reason = skipped_predicate_results
                .iter()
                .find(|predicate| predicate.name == func.name)
                .and_then(|predicate| predicate.reason.clone());
            self.result
                .invariant_skip_with_predicates(SkipReason(skip_reason), skipped_predicate_results);
            return self.result;
        }
        // Predicates stay in source declaration order; `func` anchors the campaign when it is
        // live.
        let anchor_idx =
            live_invariants.iter().position(|(invariant, _)| *invariant == func).unwrap_or(0);

        let mut executor = self.clone_executor();
        // Enable edge coverage if running with coverage guided fuzzing or with edge coverage
        // metrics (useful for benchmarking the fuzzer).
        executor.inspector_mut().collect_edge_coverage_with_config(&invariant_config.corpus);
        executor
            .inspector_mut()
            .collect_sancov_edges(invariant_config.corpus.collect_sancov_edges());
        executor
            .inspector_mut()
            .collect_sancov_trace_cmp(invariant_config.corpus.collect_sancov_trace_cmp());
        let mut config = invariant_config.clone();
        let failure_dir = invariant_suite_paths(
            &mut config.corpus,
            invariant_config.failure_persist_dir.clone().unwrap(),
            self.cr.name,
            &func.name,
            is_optimization,
        );
        // Snapshot the per-test corpus dir before `config` is moved into `InvariantExecutor`.
        let resolved_corpus_dir = config.corpus.corpus_dir.clone();

        let mut evm = InvariantExecutor::new_with_fuzz_seed(
            executor,
            self.invariant_runner(),
            self.config.fuzz.seed,
            config,
            identified_contracts,
            &self.cr.mcr.known_contracts,
            self.cr.num_invariant_campaign_anchors,
        );

        let predicate_count = live_invariants.len() + skipped_predicate_results.len();
        let invariant_contract = InvariantContract::new(
            self.address,
            self.cr.name,
            live_invariants,
            anchor_idx,
            call_after_invariant,
            &self.cr.contract.abi,
        );
        let anchor = invariant_contract.anchor();
        let is_campaign = predicate_count > 1;
        let invariant_count = is_campaign.then_some(predicate_count);
        let invariant_display_name = if is_campaign {
            Cow::Owned(invariant_campaign_display_name(self.cr.name))
        } else {
            Cow::Borrowed(func.name.as_str())
        };

        // Select the per-test targets once; the campaign, replay and symbolic paths all need the
        // same selection and settings.
        if let Err(e) = evm.select_contract_artifacts(self.address) {
            self.result.invariant_setup_fail(e);
            return self.result;
        }
        let (sender_filters, targeted) = match evm.select_contracts_and_senders(self.address) {
            Ok(selected) => selected,
            Err(e) => {
                self.result.invariant_setup_fail(e);
                return self.result;
            }
        };
        let current_settings = InvariantSettings::new(
            &targeted.targets(),
            &sender_filters,
            invariant_config.fail_on_revert,
        );

        let showmap = self.cr.mcr.tcfg.showmap.as_ref();
        let minimize = self.cr.mcr.tcfg.fuzz_minimize.as_ref();
        if showmap.is_some() || minimize.is_some() {
            let dynamic = evm.dynamic_target_ctx();
            let replay_target = ShowmapReplayTarget {
                stateless: None,
                fuzz_fail_on_revert: false,
                fuzzed_contracts: Some(&targeted),
                invariant_address: Some(self.address),
                invariant_fns: &invariant_contract.invariant_fns,
                invariant_replay: InvariantReplayOptions {
                    check_interval: invariant_config.check_interval,
                    call_after_invariant,
                    is_optimization,
                },
                dynamic: Some(&dynamic),
            };
            // Showmap replay mode: replay the persisted corpus and emit coverage files instead
            // of running the invariant campaign.
            if let Some(showmap) = showmap {
                let corpus_dir = showmap
                    .corpus_dir
                    .clone()
                    .map(|corpus_dir| {
                        let target_dir = invariant_corpus_dir(
                            &corpus_dir,
                            self.cr.name,
                            &func.name,
                            is_optimization,
                        );
                        narrow_generated_corpus_root(corpus_dir, target_dir)
                    })
                    .or(resolved_corpus_dir);
                return self.run_showmap(func, &func.name, corpus_dir, showmap, replay_target);
            }
            if let Some(minimize) = minimize {
                let target = self.fuzz_minimize_target_id(&invariant_display_name);
                replay_fuzz_minimize(
                    &mut self.result,
                    minimize,
                    target,
                    &evm.executor,
                    &invariant_config.corpus,
                    replay_target,
                );
                return self.result;
            }
        }

        let progress = self.fuzz_progress(
            &invariant_display_name,
            invariant_config.timeout,
            invariant_config.runs,
        );
        let primary_failure_file = invariant_failure_file(&failure_dir, anchor);

        // Try to replay recorded failure if any. `forge fuzz replay` checks each selected
        // predicate as the replay anchor because merged invariant suites persist failures per
        // predicate, while campaign runs use a stable suite anchor.
        let mut replayed_persisted_invariant = false;
        let mut replayed_secondary_failures = Vec::new();
        let replay_candidates = invariant_contract
            .invariant_fns
            .iter()
            .copied()
            .sorted_by_key(|(invariant, _)| (*invariant == anchor) == fuzz_failure_replay)
            .collect::<Vec<_>>();
        for (replay_invariant, fail_on_revert) in replay_candidates {
            let Some(InvariantPersistedFailure {
                mut call_sequence,
                assertion_failure,
                storage,
                failure_site,
                ..
            }) = persisted_invariant_failure(&failure_dir, replay_invariant, &current_settings)
            else {
                continue;
            };
            replayed_persisted_invariant = true;
            let replay_anchor_idx = invariant_contract
                .invariant_fns
                .iter()
                .position(|(invariant, _)| *invariant == replay_invariant)
                .expect("replay anchor must be present in invariant_fns");
            let replay_contract = InvariantContract::new(
                self.address,
                self.cr.name,
                invariant_contract.invariant_fns.clone(),
                replay_anchor_idx,
                call_after_invariant,
                &self.cr.contract.abi,
            );
            let Ok((txes, mut replay)) = self.replay_persisted_call_sequence(
                &replay_contract,
                &mut call_sequence,
                assertion_failure,
                &storage,
            ) else {
                continue;
            };
            if replay.success {
                continue;
            }
            let Some(confirmed_failure_site) =
                replay.failure_site.map(SymbolicInvariantFailureSite::from)
            else {
                continue;
            };
            if failure_site.is_some_and(|expected| expected != confirmed_failure_site) {
                continue;
            }
            if replay_invariant != anchor && !fuzz_failure_replay {
                let is_revert = match confirmed_failure_site {
                    SymbolicInvariantFailureSite::Invariant { selector, .. }
                        if selector == replay_invariant.selector() =>
                    {
                        false
                    }
                    SymbolicInvariantFailureSite::SequenceCall { .. }
                        if fail_on_revert && !replay.sequence_assertion_failure =>
                    {
                        true
                    }
                    _ => continue,
                };
                replayed_secondary_failures.push((
                    replay_invariant.name.clone(),
                    InvariantFuzzError::from_replayed_invariant(
                        self.address,
                        replay_invariant,
                        txes,
                        replay.reason,
                        invariant_config,
                        fail_on_revert,
                        assertion_failure,
                        is_revert,
                    ),
                    storage,
                    confirmed_failure_site,
                ));
                continue;
            }
            let warn = "Replayed invariant failure from persisted file. \nRun `forge clean` or remove file to ignore failure and to continue invariant test campaign.";
            if let Some(progress) = &progress {
                progress.set_prefix(format!("{invariant_display_name}\n{warn}\n"));
            } else {
                let _ = sh_warn!("{warn}");
            }

            // If sequence still fails then replay error to collect traces and exit without
            // executing new runs.
            let trace_executor = match self.clone_executor_with_symbolic_storage(&storage) {
                Ok(executor) => executor,
                Err(err) => {
                    error!(%err, "Failed to apply symbolic storage for invariant error replay");
                    self.result.single_fail(Some(err.to_string()));
                    return self.result;
                }
            };
            match self.replay_error(
                invariant_config.clone(),
                trace_executor,
                &txes,
                None,
                assertion_failure,
                None,
                &replay_contract,
                replay_invariant,
                identified_contracts,
                progress.as_ref(),
                None,
            ) {
                Ok(ReplayErrorResult {
                    counterexample_sequence: sequence, check_result, ..
                }) if !sequence.is_empty() => {
                    call_sequence = sequence;
                    if let Some(updated) = check_result {
                        if updated.failure_site.map(SymbolicInvariantFailureSite::from)
                            != Some(confirmed_failure_site)
                        {
                            continue;
                        }
                        replay = updated;
                    }
                    record_invariant_failure(
                        &invariant_failure_file(&failure_dir, replay_invariant),
                        &call_sequence,
                        &current_settings,
                        assertion_failure,
                        &storage,
                        Some(confirmed_failure_site),
                    );
                }
                Ok(_) => {}
                Err(err) => {
                    error!(%err, "Failed to replay invariant error");
                }
            }

            self.result.invariant_replay_fail(
                replay,
                &replay_invariant.name,
                None,
                call_sequence.clone(),
            );
            let signature = replay_invariant.signature();
            if let Some(artifact) = self.persist_sequence_artifact(
                &signature,
                &format!("{signature}-replay"),
                self.sequence_calls(&call_sequence),
                self.config.invariant.fail_on_revert,
                &storage,
                Some(SymbolicInvariantArtifactFailure::Predicate {
                    name: replay_invariant.name.clone(),
                    site: Some(confirmed_failure_site),
                }),
            ) {
                self.result.add_counterexample_artifact(artifact);
            }
            return self.result;
        }

        // Replay persisted handler bugs; feed still-reproducing ones into the campaign,
        // delete stale files in place.
        let (mut persisted_handler_failures, mut symbolic_handler_storage) = self
            .replay_persisted_handler_failures(&failure_dir.join("handlers"), &current_settings);

        // `forge fuzz replay` (without `--corpus-dir`) only replays persisted failures and
        // must never start a fresh campaign. If handler bugs still reproduce, surface them
        // through the normal invariant result path below; otherwise report a skip.
        if fuzz_failure_replay && persisted_handler_failures.is_empty() {
            let reason = if replayed_persisted_invariant {
                "no persisted invariant failure reproduced for selected invariants".to_string()
            } else {
                format!("no persisted invariant failure reproduced for {}", anchor.name)
            };
            self.result.single_skip(SkipReason(Some(reason)));
            return self.result;
        }

        if self.config.symbolic.enabled && !is_optimization {
            let symbolic_targets = targeted
                .targets()
                .iter()
                .flat_map(|(address, contract)| {
                    let contract_name = Some(contract.identifier.clone());
                    contract.abi_fuzzed_functions().map(move |function| SymbolicInvariantTarget {
                        address: *address,
                        contract_name: contract_name.clone(),
                        function: function.clone(),
                    })
                })
                .collect::<Vec<_>>();
            let after_invariant = call_after_invariant
                .then(|| {
                    self.cr
                        .contract
                        .abi
                        .functions()
                        .find(|func| func.name == "afterInvariant" && func.inputs.is_empty())
                })
                .flatten();
            let unsupported_domain_reason = symbolic_invariant_unsupported_domain_reason(
                invariant_config,
                &sender_filters,
                &targeted,
                &symbolic_targets,
            );

            let anchor_fail_on_revert = invariant_contract.invariant_fns[anchor_idx].1;
            let mut symbolic_invariant_config = invariant_config.clone();
            symbolic_invariant_config.fail_on_revert = anchor_fail_on_revert;
            let symbolic_config = self.config.symbolic.clone();
            let incomplete = |kind, reason: &str, stats, replay| {
                SymbolicResult::incomplete(
                    &symbolic_config,
                    kind,
                    reason,
                    stats,
                    replay,
                    SymbolicCallTrace::none(),
                    None,
                )
            };
            let mut symbolic = SymbolicExecutor::new(symbolic_config.clone());
            match symbolic.run_invariant(SymbolicInvariantRunInput {
                executor: &evm.executor,
                invariant_address: self.address,
                sender: self.sender,
                invariant: anchor,
                after_invariant,
                targets: symbolic_targets,
                senders: sender_filters.targeted,
                excluded_senders: sender_filters.excluded,
                depth: symbolic_config.invariant_depth as usize,
                check_interval: invariant_config.check_interval,
                fail_on_revert: anchor_fail_on_revert,
                ffi_enabled: self.config.ffi,
            }) {
                SymbolicInvariantRunResult::Safe(stats) => {
                    self.result.record_symbolic(match unsupported_domain_reason {
                        Some(reason) => incomplete(
                            SymbolicStopReason::Stuck,
                            reason,
                            stats,
                            SymbolicReplayMetadata::not_required(),
                        ),
                        None => SymbolicResult::pass(&symbolic_config, stats),
                    });
                }
                SymbolicInvariantRunResult::Incomplete { kind, reason, stats } => {
                    self.result.record_symbolic(incomplete(
                        kind,
                        &reason,
                        stats,
                        SymbolicReplayMetadata::not_required(),
                    ));
                }
                SymbolicInvariantRunResult::Counterexample {
                    kind,
                    sequence,
                    storage,
                    stats,
                } => 'counterexample: {
                    let is_handler = matches!(kind, SymbolicInvariantCounterexampleKind::Handler);
                    let symbolic_calls = symbolic_invariant_counterexample_calls(
                        &sequence,
                        identified_contracts,
                        invariant_config.show_solidity,
                    );
                    let check = SequenceReplay {
                        invariant_config: &symbolic_invariant_config,
                        invariant_contract: &invariant_contract,
                        target_invariant: anchor,
                        assertion_failure: false,
                        storage: &storage,
                    };
                    let replayed = self
                        .symbolic_sequence_failure(check, &symbolic_calls)
                        .ok_or("symbolic invariant counterexample did not replay")
                        .and_then(|failure| {
                            let handler_site = match failure.failure_site {
                                Some(CheckSequenceFailureSite::SequenceCall {
                                    target,
                                    selector,
                                    fingerprint,
                                }) if is_handler => Some((target, selector, fingerprint)),
                                _ => None,
                            };
                            if is_handler && handler_site.is_none() {
                                return Err("symbolic handler counterexample replayed at a \
                                            non-handler failure site");
                            }
                            Ok((failure, handler_site))
                        });
                    let (failure, handler_site) = match replayed {
                        Ok(replayed) => replayed,
                        Err(reason) => {
                            self.result.record_symbolic(incomplete(
                                SymbolicStopReason::Error,
                                reason,
                                stats,
                                SymbolicReplayMetadata::mismatch(reason.to_string()),
                            ));
                            break 'counterexample;
                        }
                    };

                    let txes = symbolic_calls
                        .iter()
                        .map(SymbolicCounterexampleCall::to_basic_tx_details)
                        .collect::<Vec<_>>();
                    let original_sequence_len = txes.len();
                    let failure_site = failure.failure_site.map(SymbolicInvariantFailureSite::from);
                    let (artifact_file_name, artifact_failure) = match handler_site {
                        Some((reverter, selector, fingerprint)) => (
                            format!("handler-{reverter}-{selector}"),
                            SymbolicInvariantArtifactFailure::Handler {
                                name: Some(invariant_handler_failure_name(
                                    identified_contracts,
                                    reverter,
                                    selector,
                                )),
                                reverter,
                                selector,
                                fingerprint,
                            },
                        ),
                        None => (
                            anchor.signature(),
                            SymbolicInvariantArtifactFailure::Predicate {
                                name: anchor.name.clone(),
                                site: failure_site,
                            },
                        ),
                    };
                    let replayed = match self.replay_invariant_error_sequence(
                        SequenceReplay { assertion_failure: is_handler, ..check },
                        &txes,
                        None,
                        identified_contracts,
                        &current_settings,
                        SequenceArtifactSpec {
                            file_name: &artifact_file_name,
                            fail_on_revert: is_handler || anchor_fail_on_revert,
                            failure: Some(artifact_failure),
                        },
                        progress.as_ref(),
                        Some((1, 1)),
                    ) {
                        Ok(replayed) => replayed,
                        Err(err) => {
                            let reason = format!("symbolic invariant replay failed: {err}");
                            self.result.record_symbolic(incomplete(
                                SymbolicStopReason::Error,
                                &reason,
                                stats,
                                SymbolicReplayMetadata::error(reason.clone()),
                            ));
                            break 'counterexample;
                        }
                    };
                    let ReplayedInvariantSequence {
                        call_sequence,
                        artifact,
                        minimization,
                        fork_block_number,
                    } = replayed;
                    let mut symbolic_result = SymbolicResult::fail_counterexample_sequence(
                        &symbolic_config,
                        stats,
                        SymbolicCallTrace::test_result_traces(!self.result.traces.is_empty()),
                    );
                    if let Some(artifact) = artifact.clone() {
                        symbolic_result = symbolic_result.with_artifact(artifact);
                    }
                    if let Some(minimization) = minimization.clone() {
                        symbolic_result = symbolic_result.with_minimization(minimization);
                    }
                    let reason = failure.reason.unwrap_or_else(|| {
                        if is_handler {
                            "symbolic handler counterexample".to_string()
                        } else {
                            "symbolic invariant counterexample".to_string()
                        }
                    });

                    if let Some((reverter, selector, fingerprint)) = handler_site {
                        let call_sequence =
                            call_sequence.iter().map(base_counterexample_to_tx).collect::<Vec<_>>();
                        symbolic_handler_storage.insert(
                            (reverter, selector, fingerprint),
                            SymbolicHandlerReplayStorage {
                                call_sequence: call_sequence.clone(),
                                assignments: storage,
                            },
                        );
                        persisted_handler_failures.insert(
                            (reverter, selector),
                            InvariantFuzzError::HandlerAssertion(HandlerAssertionFailure {
                                reverter,
                                selector,
                                call_sequence,
                                original_sequence_len,
                                revert_reason: reason,
                                fork_block_number: None,
                                edge_fingerprint: fingerprint,
                            }),
                        );
                        self.result.record_symbolic(symbolic_result);
                        break 'counterexample;
                    }

                    record_invariant_failure(
                        &primary_failure_file,
                        &call_sequence,
                        &current_settings,
                        false,
                        &storage,
                        failure_site,
                    );
                    let mut invariant_failures = vec![InvariantFailure::Predicate {
                        name: anchor.name.clone(),
                        reason,
                        counterexample: Some(CounterExample::Sequence(
                            original_sequence_len,
                            call_sequence,
                        )),
                        artifact,
                        minimization,
                        persisted_path: primary_failure_file,
                        is_anchor: true,
                    }];
                    for (invariant, _) in &invariant_contract.invariant_fns {
                        if let Some((_, error, _, _)) = replayed_secondary_failures
                            .iter()
                            .find(|(name, ..)| name == &invariant.name)
                            && let Some(calls) = failed_invariant_calls(error)
                        {
                            invariant_failures.push(InvariantFailure::Predicate {
                                name: invariant.name.clone(),
                                reason: error.revert_reason().unwrap_or_default(),
                                counterexample: Some(CounterExample::Sequence(
                                    calls.len(),
                                    base_counterexamples(
                                        calls,
                                        identified_contracts,
                                        invariant_config.show_solidity,
                                    ),
                                )),
                                artifact: None,
                                minimization: None,
                                persisted_path: invariant_failure_file(&failure_dir, invariant),
                                is_anchor: false,
                            });
                        }
                    }
                    let invariant_predicate_results = if is_campaign {
                        self.sort_predicate_results(
                            invariant_failures
                                .iter()
                                .map(|failure| InvariantPredicateResult {
                                    name: failure.name().to_string(),
                                    status: TestStatus::Failure,
                                    reason: Some(failure.reason().to_string()),
                                })
                                .chain(skipped_predicate_results),
                        )
                    } else {
                        Vec::new()
                    };
                    self.result.invariant_result(
                        invariant_kind(1, failure.calls_count, failure.reverts),
                        InvariantOutcome {
                            fork_block_number,
                            failures: invariant_failures,
                            predicate_results: invariant_predicate_results,
                            failure_dir: Some(failure_dir),
                            invariant_count,
                            ..Default::default()
                        },
                    );
                    self.result.record_symbolic(symbolic_result);
                    return self.result;
                }
            }
        }

        let mut invariant_result = match evm.invariant_fuzz(
            invariant_contract.clone(),
            &self.setup.fuzz_fixtures,
            self.build_fuzz_state(true, None),
            progress.as_ref(),
            &self.tcfg.early_exit,
            persisted_handler_failures,
        ) {
            Ok(x) => x,
            Err(e) => {
                self.result.invariant_setup_fail(e);
                return self.result;
            }
        };
        let mut replayed_secondary_metadata = BTreeMap::new();
        for (name, failure, storage, failure_site) in replayed_secondary_failures {
            if let Entry::Vacant(entry) = invariant_result.errors.entry(name) {
                replayed_secondary_metadata.insert(entry.key().clone(), (storage, failure_site));
                entry.insert(failure);
            }
        }
        // Merge coverage collected during invariant run with test setup coverage.
        self.result.merge_coverages(invariant_result.line_coverage);

        let mut counterexample = None;
        // Success requires zero predicate breaks *and* zero handler-side assertion bugs.
        let success =
            invariant_result.errors.is_empty() && invariant_result.handler_errors.is_empty();
        let single_failure =
            invariant_result.errors.len() + invariant_result.handler_errors.len() == 1;
        let mut fork_block_number = invariant_result.fork_block_number;
        let mut invariant_failures = Vec::new();
        let mut any_failure_persisted = false;

        if success {
            if let Some(best_value) = invariant_result.optimization_best_value {
                // Optimization mode: replay and shrink to find shortest best sequence.
                match self.replay_error(
                    invariant_config.clone(),
                    self.clone_executor(),
                    &invariant_result.optimization_best_sequence,
                    None,
                    false,
                    Some(best_value),
                    &invariant_contract,
                    anchor,
                    identified_contracts,
                    progress.as_ref(),
                    None,
                ) {
                    Ok(ReplayErrorResult { counterexample_sequence: sequence, .. })
                        if !sequence.is_empty() =>
                    {
                        counterexample = Some(CounterExample::Sequence(
                            invariant_result.optimization_best_sequence.len(),
                            sequence,
                        ));
                    }
                    Err(err) => {
                        error!(%err, "Failed to replay optimization best sequence");
                    }
                    _ => {}
                }
            } else if let Err(err) = replay_run(
                // Standard check mode: replay last run for traces.
                &invariant_contract,
                anchor,
                self.clone_executor(),
                &self.cr.mcr.known_contracts,
                identified_contracts.clone(),
                &mut self.result.logs,
                &mut self.result.traces,
                &mut self.result.debug_bytecodes,
                &mut self.result.line_coverage,
                &mut self.result.deprecated_cheatcodes,
                &invariant_result.last_run_inputs,
                invariant_config.show_solidity,
            ) {
                error!(%err, "Failed to replay last invariant run");
            }
        } else {
            // Total broken invariants in this campaign, used to decorate the shrink progress bar
            // with `[i/N]`. `errors` keys cover both the anchor and any broken secondaries.
            let total_broken = invariant_result.errors.len();
            // The anchor is shrunk first (as `[1/N]`); secondaries follow and only advance the
            // counter when they are actually shrunk so it matches user-visible progress.
            let mut next_position = 2usize;
            let order = std::iter::once(anchor_idx).chain(
                (0..invariant_contract.invariant_fns.len()).filter(|idx| *idx != anchor_idx),
            );
            for idx in order {
                let is_anchor = idx == anchor_idx;
                let invariant = invariant_contract.invariant_fns[idx].0;
                let Some(error) = invariant_result.errors.get(&invariant.name) else {
                    continue;
                };
                let persisted_path = invariant_failure_file(&failure_dir, invariant);
                let (case_data, calls) = match error {
                    InvariantFuzzError::BrokenInvariant(case_data)
                    | InvariantFuzzError::Revert(case_data) => {
                        (case_data, failed_invariant_calls(error).unwrap_or_default())
                    }
                    // Non-replayable anchor errors (e.g. `MaxAssumeRejects`) still get an entry,
                    // without a counterexample, so the reason is rendered.
                    _ if is_anchor => {
                        invariant_failures.push(InvariantFailure::Predicate {
                            name: invariant.name.clone(),
                            reason: error.revert_reason().unwrap_or_default(),
                            counterexample: None,
                            artifact: None,
                            minimization: None,
                            persisted_path,
                            is_anchor,
                        });
                        continue;
                    }
                    _ => continue,
                };
                let replayed_metadata = replayed_secondary_metadata.get(&invariant.name);

                // On Ctrl+C: skip the (potentially long) secondary replay+shrink, but still
                // persist the un-shrunk sequence so the next run targeting this invariant picks
                // it up and shrinks from the saved counterexample. The current run's output
                // still gets a terse `name: reason` line via the no-counterexample path.
                let replayed = if !is_anchor && self.tcfg.early_exit.should_stop() {
                    if replayed_metadata.is_none() {
                        record_invariant_failure(
                            &persisted_path,
                            &base_counterexamples(
                                calls,
                                identified_contracts,
                                invariant_config.show_solidity,
                            ),
                            &current_settings,
                            case_data.assertion_failure,
                            &[],
                            None,
                        );
                    }
                    any_failure_persisted = true;
                    None
                } else {
                    let position = if is_anchor {
                        1
                    } else {
                        next_position += 1;
                        next_position - 1
                    };
                    let mut replay_config = invariant_config.clone();
                    if replayed_metadata.is_some() {
                        // The persisted failure site was validated before entering the
                        // campaign. The generic shrinker only preserves failure, not its site,
                        // so shrinking here could misattribute a different failure to this
                        // predicate.
                        replay_config.shrink_run_limit = 0;
                    }
                    let (storage, artifact_failure) = match replayed_metadata {
                        Some((storage, site)) => (
                            storage.as_slice(),
                            Some(SymbolicInvariantArtifactFailure::Predicate {
                                name: invariant.name.clone(),
                                site: Some(*site),
                            }),
                        ),
                        None => (&[][..], None),
                    };
                    let signature = invariant.signature();
                    match self.replay_invariant_error_sequence(
                        SequenceReplay {
                            invariant_config: &replay_config,
                            invariant_contract: &invariant_contract,
                            target_invariant: invariant,
                            assertion_failure: case_data.assertion_failure,
                            storage,
                        },
                        calls,
                        Some(case_data.inner_sequence.clone()),
                        identified_contracts,
                        &current_settings,
                        SequenceArtifactSpec {
                            file_name: &signature,
                            fail_on_revert: self.config.invariant.fail_on_revert,
                            failure: artifact_failure,
                        },
                        progress.as_ref(),
                        Some((position, total_broken)),
                    ) {
                        Ok(replayed) if !replayed.call_sequence.is_empty() => {
                            if single_failure {
                                fork_block_number =
                                    replayed.fork_block_number.or(fork_block_number);
                            }
                            // Keep all replay metadata for a seeded persisted failure. A fresh
                            // campaign error takes precedence and is persisted normally.
                            if replayed_metadata.is_none() {
                                record_invariant_failure(
                                    &persisted_path,
                                    &replayed.call_sequence,
                                    &current_settings,
                                    case_data.assertion_failure,
                                    &[],
                                    None,
                                );
                            }
                            any_failure_persisted = true;
                            Some(replayed)
                        }
                        Ok(_) => None,
                        Err(err) => {
                            error!(%err, "Failed to replay invariant error");
                            None
                        }
                    }
                };
                let (counterexample, artifact, minimization) = match replayed {
                    Some(replayed) => (
                        Some(CounterExample::Sequence(calls.len(), replayed.call_sequence)),
                        replayed.artifact,
                        replayed.minimization,
                    ),
                    None => (None, None, None),
                };
                invariant_failures.push(InvariantFailure::Predicate {
                    name: invariant.name.clone(),
                    reason: error.revert_reason().unwrap_or_default(),
                    counterexample,
                    artifact,
                    minimization,
                    persisted_path,
                    is_anchor,
                });
            }
        }

        let invariant_failure_dir = any_failure_persisted.then(|| failure_dir.clone());
        let invariant_predicate_results = if is_campaign {
            let failures_by_name = invariant_failures
                .iter()
                .map(|failure| (failure.name(), failure))
                .collect::<BTreeMap<_, _>>();
            self.sort_predicate_results(
                invariant_contract
                    .invariant_fns
                    .iter()
                    .map(|(invariant, _)| {
                        let failure = failures_by_name.get(invariant.name.as_str());
                        InvariantPredicateResult {
                            name: invariant.name.clone(),
                            status: if failure.is_some() {
                                TestStatus::Failure
                            } else {
                                TestStatus::Success
                            },
                            reason: failure.map(|failure| failure.reason().to_string()),
                        }
                    })
                    .chain(skipped_predicate_results),
            )
        } else {
            Vec::new()
        };

        // Convert handler-side assertion bugs into render-ready entries. The name is a
        // best-effort `Contract::function` from `identified_contracts`, falling back to
        // `0xreverter::0xselector`. Map is keyed by `(reverter, selector)` site so multiple
        // code paths through the same function collapse to one entry, rendered in the
        // dedicated handler assertions section.
        let invariant_handler_failures = invariant_result
            .handler_errors
            .iter()
            // Stable order across runs: sort by `(reverter, selector)` site directly.
            .sorted_by(|(a, _), (b, _)| a.cmp(b))
            .filter_map(|(_, err)| err.as_handler_assertion())
            .map(|failure| {
                let (reverter, selector) = (failure.reverter, failure.selector);
                let name = invariant_handler_failure_name(identified_contracts, reverter, selector);
                let symbolic_storage = symbolic_handler_storage
                    .get(&(reverter, selector, failure.edge_fingerprint))
                    .filter(|storage| storage.call_sequence == failure.call_sequence)
                    .map_or(&[][..], |storage| &storage.assignments);
                let calls = base_counterexamples(
                    &failure.call_sequence,
                    identified_contracts,
                    invariant_config.show_solidity,
                );

                // Persist for next-run replay (skip if nothing to record).
                if !calls.is_empty() {
                    record_handler_failure(
                        &failure_dir,
                        reverter,
                        selector,
                        failure.edge_fingerprint,
                        &calls,
                        &current_settings,
                        symbolic_storage,
                    );
                }
                let artifact = self.persist_sequence_artifact(
                    &anchor.signature(),
                    &format!("handler-{reverter}-{selector}"),
                    self.sequence_calls(&calls),
                    true,
                    symbolic_storage,
                    Some(SymbolicInvariantArtifactFailure::Handler {
                        name: Some(name.clone()),
                        reverter,
                        selector,
                        fingerprint: failure.edge_fingerprint,
                    }),
                );
                // Preserve pre-shrink length for `(original: N, shrunk: M)` rendering.
                let counterexample = (!calls.is_empty())
                    .then(|| CounterExample::Sequence(failure.original_sequence_len, calls));

                InvariantFailure::Handler {
                    name,
                    reverter,
                    selector,
                    reason: failure.revert_reason.clone(),
                    counterexample,
                    artifact,
                }
            })
            .collect::<Vec<_>>();

        self.result.invariant_result(
            TestKind::Invariant {
                runs: invariant_result.runs,
                calls: invariant_result.calls,
                reverts: invariant_result.reverts,
                workers: invariant_result.workers.max(1),
                metrics: invariant_result.metrics,
                failed_corpus_replays: invariant_result.failed_corpus_replays,
                optimization_best_value: invariant_result.optimization_best_value,
            },
            InvariantOutcome {
                success,
                fork_block_number,
                failures: invariant_failures,
                handler_failures: invariant_handler_failures,
                predicate_results: invariant_predicate_results,
                failure_dir: invariant_failure_dir,
                invariant_count,
                counterexample,
                gas_report_traces: invariant_result.gas_report_traces,
            },
        );
        self.result
    }

    /// Orders predicate results by their declaration position in the test contract ABI.
    fn sort_predicate_results(
        &self,
        results: impl Iterator<Item = InvariantPredicateResult>,
    ) -> Vec<InvariantPredicateResult> {
        results
            .sorted_by_key(|predicate| {
                self.cr
                    .contract
                    .abi
                    .functions()
                    .position(|func| func.name == predicate.name)
                    .unwrap_or(usize::MAX)
            })
            .collect()
    }

    fn invariant_skip_reason(&self, func: &Function) -> Option<SkipReason> {
        match self.executor.call(
            self.sender,
            self.address,
            func,
            &[],
            U256::ZERO,
            Some(self.revert_decoder()),
        ) {
            Err(EvmError::Skip(reason)) => Some(reason),
            _ => None,
        }
    }

    /// Runs a fuzzed test.
    ///
    /// Applies the before test txes (if any), fuzzes the current function and returns the
    /// `TestResult`.
    ///
    /// Before test txes are applied in order and state modifications committed to the EVM database
    /// (therefore the fuzz test will use the modified state).
    /// State modifications of before test txes and fuzz test are discarded after test ends,
    /// similar to `eth_call`.
    fn run_fuzz_test(mut self, func: &Function) -> TestResult {
        // Prepare fuzz test execution.
        if self.prepare_test(func).is_err() {
            return self.result;
        }

        let runner = self.fuzz_runner();
        let mut fuzz_config = self.config.fuzz.clone();
        let (test_name, legacy_corpus_dir, (failure_dir, failure_file)) =
            self.fuzz_test_paths(func, &mut fuzz_config);
        let fuzz_input = self.cr.mcr.tcfg.fuzz_input.as_ref();
        let is_explicit_target = fuzz_input
            .is_some_and(|input| input.contract == self.cr.name && input.test == func.signature());
        if is_explicit_target && fuzz_config.run.is_some() {
            self.result.fuzz_setup_fail(eyre::eyre!(
                "`--fuzz-input-file` cannot be combined with `fuzz.run`"
            ));
            return self.result;
        }

        let replay_target = ShowmapReplayTarget {
            stateless: Some(StatelessReplayTarget { function: func, address: self.address }),
            fuzz_fail_on_revert: fuzz_config.fail_on_revert,
            fuzzed_contracts: None,
            invariant_address: None,
            invariant_fns: &[],
            invariant_replay: InvariantReplayOptions::default(),
            dynamic: None,
        };
        // Showmap replay mode: replay the persisted corpus and emit coverage
        // files instead of running the fuzz campaign.
        if let Some(showmap) = self.cr.mcr.tcfg.showmap.as_ref() {
            let corpus_dir = showmap
                .corpus_dir
                .clone()
                .map(|corpus_dir| {
                    legacy_fuzz_corpus_dir(Some(&corpus_dir), self.cr.name, func, &test_name)
                        .unwrap_or_else(|| {
                            let target_dir = corpus_dir
                                .join(contract_short_name(self.cr.name))
                                .join(&*test_name);
                            narrow_generated_corpus_root(corpus_dir, target_dir)
                        })
                })
                .or(legacy_corpus_dir)
                .or_else(|| fuzz_config.corpus.corpus_dir.clone());
            return self.run_showmap(func, &test_name, corpus_dir, showmap, replay_target);
        }
        if let Some(minimize) = self.cr.mcr.tcfg.fuzz_minimize.as_ref() {
            let target = self.fuzz_minimize_target_id(&func.signature());
            replay_fuzz_minimize(
                &mut self.result,
                minimize,
                target,
                &self.executor,
                &fuzz_config.corpus,
                replay_target,
            );
            return self.result;
        }

        // Load the validated explicit input for its unique target, or fall back to this test's
        // canonical cache.
        let persisted_failure = if is_explicit_target {
            fuzz_input.map(|input| input.failure.as_ref().clone())
        } else {
            foundry_common::fs::read_json_file::<BaseCounterExample>(&failure_file).ok().or_else(
                || {
                    if test_name == func.name {
                        return None;
                    }
                    let legacy_file = canonicalized(failure_dir.join(&func.name));
                    let failure =
                        foundry_common::fs::read_json_file::<BaseCounterExample>(&legacy_file)
                            .ok()?;
                    failure
                        .calldata
                        .get(..4)
                        .is_some_and(|selector| func.selector() == selector)
                        .then_some(failure)
                },
            )
        };
        if self.cr.mcr.tcfg.fuzz_failure_replay {
            let skip_reason = match &persisted_failure {
                None => {
                    Some(format!("no persisted fuzz failure found at {}", failure_file.display()))
                }
                Some(failure)
                    if failure
                        .calldata
                        .get(..4)
                        .is_none_or(|selector| func.selector() != selector) =>
                {
                    Some(format!("persisted fuzz failure selector does not match {}", func.name))
                }
                Some(_) => None,
            };
            if let Some(reason) = skip_reason {
                self.result.fuzz_result(FuzzTestResult {
                    skipped: true,
                    reason: Some(reason),
                    ..Default::default()
                });
                return self.result;
            }
            fuzz_config.corpus.corpus_dir = None;
        }

        self.try_seed_fuzz_corpus_from_frontiers(func, &fuzz_config);
        self.try_seed_fuzz_corpus_symbolically(func, &fuzz_config);

        let progress = self.fuzz_progress(
            &func.name,
            fuzz_config.timeout,
            if fuzz_config.run.is_some() { 1 } else { fuzz_config.runs },
        );

        let state = self.build_fuzz_state(false, Some(func));
        let mut executor = self.executor.into_owned();
        // Enable edge coverage if running with coverage guided fuzzing or with edge coverage
        // metrics (useful for benchmarking the fuzzer).
        executor.inspector_mut().collect_edge_coverage_with_config(&fuzz_config.corpus);
        executor.inspector_mut().collect_evm_cmp_log(fuzz_config.corpus.collect_evm_cmp_log());
        executor.inspector_mut().collect_sancov_edges(fuzz_config.corpus.collect_sancov_edges());
        executor
            .inspector_mut()
            .collect_sancov_trace_cmp(fuzz_config.corpus.collect_sancov_trace_cmp());
        let mut fuzzed_executor = FuzzedExecutor::new(
            executor,
            runner,
            self.tcfg.sender,
            fuzz_config,
            persisted_failure,
            legacy_corpus_dir,
        );
        let result = if self.cr.mcr.tcfg.fuzz_failure_replay {
            fuzzed_executor.replay_persisted_failure(
                func,
                self.address,
                &self.cr.mcr.revert_decoder,
            )
        } else {
            fuzzed_executor.fuzz(
                func,
                &self.setup.fuzz_fixtures,
                state,
                self.address,
                &self.cr.mcr.revert_decoder,
                progress.as_ref(),
                &self.tcfg.early_exit,
                &self.cr.tokio_handle,
            )
        };
        let result = match result {
            Ok(result) => result,
            Err(e) => {
                self.result.fuzz_setup_fail(e);
                return self.result;
            }
        };

        // Record counterexample.
        if !self.cr.mcr.tcfg.fuzz_failure_replay
            && let Some(CounterExample::Single(counterexample)) = &result.counterexample
        {
            if let Err(err) = foundry_common::fs::create_dir_all(failure_dir) {
                error!(%err, "Failed to create fuzz failure dir");
            } else if let Err(err) =
                foundry_common::fs::write_json_file(&failure_file, counterexample)
            {
                error!(%err, "Failed to record call sequence");
            }
        }

        self.result.fuzz_result(result);
        self.result
    }

    fn prepare_test(&mut self, func: &Function) -> Result<(), ()> {
        let address = self.setup.address;

        // Apply before test configured functions (if any).
        if self.cr.contract.abi.functions().any(|func| func.name.is_before_test_setup()) {
            for calldata in self.executor.call_sol_default(
                address,
                &ITest::beforeTestSetupCall { testSelector: func.selector() },
            ) {
                let spec_id: SpecId = self.executor.spec_id().into();
                debug!(?calldata, spec=%spec_id, "applying before_test_setup");
                // Apply before test configured calldata.
                let Ok(call_result) = self.executor.to_mut().transact_raw(
                    self.tcfg.sender,
                    address,
                    calldata,
                    U256::ZERO,
                ) else {
                    self.result.single_fail(None);
                    return Err(());
                };
                let reverted = call_result.reverted;
                // Merge tx result traces in unit test result.
                self.result.extend_setup(call_result);
                // To continue unit test execution the call should not revert.
                if reverted {
                    self.result.single_fail(None);
                    return Err(());
                }
            }
        }
        Ok(())
    }

    fn fuzz_runner(&self) -> TestRunner {
        let config = &self.config.fuzz;
        fuzzer_with_cases(config.seed, config.runs, config.max_test_rejects)
    }

    /// Replays the persisted corpus and writes AFL-`afl-showmap`-style files.
    fn run_showmap(
        mut self,
        func: &Function,
        test_name: &str,
        corpus_dir: Option<PathBuf>,
        showmap: &crate::multi_runner::ShowmapConfig,
        target: ShowmapReplayTarget<'_>,
    ) -> TestResult {
        let Some(corpus_dir) = corpus_dir else {
            self.result.replay_skip("no corpus_dir configured for this test");
            return self.result;
        };

        // Configure executor with the requested coverage collectors. Showmap
        // ignores fuzz config defaults: the CLI domain is the source of truth.
        // For EVM we enable line coverage rather than edge coverage so the IDs
        // (bytecode_hash, pc) are deterministic across forge processes —
        // `EdgeCovInspector` uses a per-process random hash and would yield
        // non-comparable IDs across approaches.
        let mut executor = self.clone_executor();
        let domain = showmap.domain;
        executor.inspector_mut().collect_line_coverage(domain.includes_evm());
        executor.inspector_mut().collect_sancov_edges(domain.includes_sancov());

        // Fold test identity into the approach dir so each `<approach>/` contains
        // trials of a single test — what `differential-coverage` expects. The
        // (anchor) function name is included for invariant tests too so contracts
        // with multiple invariant campaigns don't collide on the same approach dir
        // (which `File::create_new` would reject). Distinct anchors sharing one
        // corpus simply produce equivalent, separately-named approach dirs.
        let safe_id = self.cr.name.replace(['/', '\\', ':'], "_");
        let safe_fn = test_name.replace(['/', '\\', ':', '(', ')', ',', ' '], "_");
        let approach = format!("{}__{safe_id}__{safe_fn}", showmap.approach);
        let opts = ShowmapOpts {
            out_dir: showmap.out_dir.clone(),
            approach,
            trial: showmap.trial.clone(),
            per_input: showmap.per_input,
            domain,
            emit_files: showmap.emit_files,
        };

        let start = std::time::Instant::now();
        let result = replay_corpus_to_showmap(&executor, &corpus_dir, target, &opts);
        let duration = start.elapsed();
        match result {
            Ok(stats) => {
                if stats.sancov_requested && !stats.sancov_observed && stats.corpus_entries > 0 {
                    let _ = sh_warn!(
                        "{}::{}: sancov coverage requested but no hits observed (build is likely not sancov-instrumented)",
                        self.cr.name,
                        func.name,
                    );
                }
                if stats.unreadable_entries > 0 {
                    self.result.single_fail(Some(format!(
                        "failed to read {} corpus entries from {}",
                        stats.unreadable_entries,
                        corpus_dir.display()
                    )));
                } else if !showmap.emit_files && stats.corpus_entries == 0 {
                    self.result.replay_skip(format!(
                        "replayed 0 corpus entries from {}",
                        corpus_dir.display()
                    ));
                } else {
                    self.result.replay_result(
                        stats.corpus_entries,
                        stats.showmap_files,
                        stats.skipped_entries,
                        duration,
                    );
                }
            }
            Err(e) => {
                self.result.single_fail(Some(e.to_string()));
            }
        }
        self.result
    }

    fn invariant_runner(&self) -> TestRunner {
        let config = &self.config.invariant;
        fuzzer_with_cases(self.config.fuzz.seed, config.runs, config.max_assume_rejects)
    }

    fn clone_executor(&self) -> Executor<FEN> {
        self.executor.clone().into_owned()
    }

    fn clone_executor_with_symbolic_storage(
        &self,
        storage: &[SymbolicStorageAssignment],
    ) -> Result<Executor<FEN>> {
        let mut executor = self.clone_executor();
        for assignment in storage {
            executor.set_storage_slot(assignment.address, assignment.slot, assignment.value)?;
            if let Some(cheats) = executor.inspector_mut().cheatcodes.as_mut() {
                cheats.cache_arbitrary_storage_value(
                    assignment.address,
                    assignment.slot,
                    assignment.value,
                );
            }
        }
        Ok(executor)
    }

    fn build_fuzz_state(&self, invariant: bool, func: Option<&Function>) -> EvmFuzzState {
        let config =
            if invariant { self.config.invariant.dictionary } else { self.config.fuzz.dictionary };
        let has_function_inline_config =
            func.is_some_and(|func| self.inline_config.contains_function(self.cr.name, &func.name));
        let can_reuse_setup_state = !invariant
            && config == self.cr.config.fuzz.dictionary
            && !has_function_inline_config
            && !self.cr.contract.abi.functions().any(|func| func.name.is_before_test_setup());
        if can_reuse_setup_state {
            return self
                .setup
                .fuzz_state
                .get_or_init(|| self.build_fuzz_state_uncached(false, config))
                .fork();
        }

        self.build_fuzz_state_uncached(invariant, config)
    }

    fn build_fuzz_state_uncached(
        &self,
        invariant: bool,
        config: FuzzDictionaryConfig,
    ) -> EvmFuzzState {
        let literals =
            if invariant { &self.cr.mcr.invariant_literals } else { &self.cr.mcr.fuzz_literals };
        if let Some(db) = self.executor.backend().active_fork_db() {
            EvmFuzzState::new(&self.setup.deployed_libs, db, config, Some(literals))
        } else {
            let db = self.executor.backend().mem_db();
            EvmFuzzState::new(&self.setup.deployed_libs, db, config, Some(literals))
        }
    }
}

fn fuzzer_with_cases(seed: Option<U256>, cases: u32, max_global_rejects: u32) -> TestRunner {
    let config = proptest::test_runner::Config {
        cases,
        max_global_rejects,
        // Disable proptest shrink: for fuzz tests we provide single counterexample,
        // for invariant tests we shrink outside proptest.
        max_shrink_iters: 0,
        ..Default::default()
    };

    if let Some(seed) = seed {
        trace!(target: "forge::test", %seed, "building deterministic fuzzer");
        let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>());
        TestRunner::new_with_rng(config, rng)
    } else {
        trace!(target: "forge::test", "building stochastic fuzzer");
        TestRunner::new(config)
    }
}

/// Holds data about a persisted invariant failure.
#[derive(Serialize, Deserialize)]
struct InvariantPersistedFailure {
    /// Recorded counterexample.
    call_sequence: Vec<BaseCounterExample>,
    /// Invariant settings when the counterexample was generated.
    /// Used to determine if the counterexample is still valid.
    settings: InvariantSettings,
    /// Whether the persisted failure came from a handler assertion instead of the invariant body.
    #[serde(default)]
    assertion_failure: bool,
    /// Concrete setup-storage assignments required before replaying this failure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    storage: Vec<SymbolicStorageAssignment>,
    /// Exact failure site required to accept a persisted symbolic handler rerun.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_site: Option<SymbolicInvariantFailureSite>,
}

/// Persisted handler-side assertion bugs keyed by `(reverter, selector)`.
type HandlerFailureMap = std::collections::HashMap<(Address, Selector), InvariantFuzzError>;
/// Symbolic replay storage for handler bugs keyed by `(reverter, selector, fingerprint)`.
type SymbolicHandlerStorageMap = HashMap<(Address, Selector, B256), SymbolicHandlerReplayStorage>;

/// Symbolic storage assignments that only apply when replaying the exact recorded sequence.
struct SymbolicHandlerReplayStorage {
    call_sequence: Vec<BasicTxDetails>,
    assignments: Vec<SymbolicStorageAssignment>,
}

/// Helper function to load failed call sequence from file.
/// Ignores failure if generated with different invariant settings than the current ones.
fn persisted_call_sequence(
    path: &Path,
    current_settings: &InvariantSettings,
) -> Option<InvariantPersistedFailure> {
    let persisted = foundry_common::fs::read_json_file::<InvariantPersistedFailure>(path).ok()?;
    if let Some(diff) = persisted.settings.diff(current_settings) {
        let _ = sh_warn!(
            "Failure from {path:?} file was ignored because invariant test settings have changed: {diff}"
        );
        return None;
    }
    Some(persisted)
}

/// Returns the current invariant failure cache path.
fn invariant_failure_file(failure_dir: &Path, invariant: &Function) -> PathBuf {
    canonicalized(failure_dir.join("invariants").join(&invariant.name))
}

/// Loads a persisted invariant failure from the new cache path, falling back to the legacy path.
fn persisted_invariant_failure(
    failure_dir: &Path,
    invariant: &Function,
    current_settings: &InvariantSettings,
) -> Option<InvariantPersistedFailure> {
    persisted_call_sequence(&invariant_failure_file(failure_dir, invariant), current_settings)
        .or_else(|| {
            // Older Foundry versions stored invariant failures directly under the failure root.
            let legacy_path = canonicalized(failure_dir.join(&invariant.name));
            let persisted = persisted_call_sequence(&legacy_path, current_settings)?;
            let _ = sh_warn!(
                "Using legacy invariant failure cache at {}; new failures will be persisted under {}/invariants.",
                legacy_path.display(),
                failure_dir.display(),
            );
            Some(persisted)
        })
}

/// Converts a persisted counterexample to `BasicTxDetails`, setting `show_solidity` in place.
fn base_counterexamples_to_txes(
    call_sequence: &mut [BaseCounterExample],
    show_solidity: bool,
) -> Vec<BasicTxDetails> {
    call_sequence
        .iter_mut()
        .map(|seq| {
            seq.show_solidity = show_solidity;
            base_counterexample_to_tx(seq)
        })
        .collect()
}

/// Converts campaign transactions into displayable counterexample calls.
fn base_counterexamples(
    calls: &[BasicTxDetails],
    identified_contracts: &ContractsByAddress,
    show_solidity: bool,
) -> Vec<BaseCounterExample> {
    calls
        .iter()
        .map(|tx| {
            BaseCounterExample::from_invariant_call(tx, identified_contracts, None, show_solidity)
        })
        .collect()
}

/// Returns the failing call sequence of a replayable invariant error.
fn failed_invariant_calls(error: &InvariantFuzzError) -> Option<&[BasicTxDetails]> {
    match error {
        InvariantFuzzError::BrokenInvariant(case_data) | InvariantFuzzError::Revert(case_data) => {
            let TestError::Fail(_, calls) = &case_data.test_error else {
                unreachable!("FailedInvariantCaseData::new always sets TestError::Fail")
            };
            Some(calls)
        }
        _ => None,
    }
}

fn base_counterexample_to_tx(seq: &BaseCounterExample) -> BasicTxDetails {
    BasicTxDetails {
        warp: seq.warp,
        roll: seq.roll,
        sender: seq.sender.unwrap_or_default(),
        call_details: CallDetails {
            target: seq.addr.unwrap_or_default(),
            calldata: seq.calldata.clone(),
            value: seq.value,
        },
    }
}

fn symbolic_invariant_counterexample_calls(
    steps: &[SymbolicInvariantStep],
    identified_contracts: &ContractsByAddress,
    show_solidity: bool,
) -> Vec<SymbolicCounterexampleCall> {
    steps
        .iter()
        .map(|step| {
            let tx = BasicTxDetails {
                warp: None,
                roll: None,
                sender: step.sender,
                call_details: CallDetails {
                    target: step.address,
                    calldata: step.calldata.clone(),
                    value: None,
                },
            };
            let counterexample = BaseCounterExample::from_invariant_call(
                &tx,
                identified_contracts,
                None,
                show_solidity,
            );
            SymbolicCounterexampleCall::from_base_counterexample(
                &counterexample,
                step.sender,
                step.address,
            )
        })
        .collect()
}

fn frontier_selector(frontier: &FuzzBranchFrontierRecord) -> Option<Selector> {
    frontier
        .sequence
        .get(frontier.call_index)
        .and_then(|call| call.call_details.calldata.get(..4))
        .map(Selector::from_slice)
}

fn parse_frontier_selectors(selectors: &[String], signature: &str) -> Vec<Selector> {
    selectors
        .iter()
        .filter_map(|selector| {
            let parsed = hex::decode(selector.strip_prefix("0x").unwrap_or(selector))
                .ok()
                .filter(|bytes| bytes.len() == 4)
                .map(|bytes| Selector::from_slice(&bytes));
            if parsed.is_none() {
                let _ = sh_warn!(
                    "invalid symbolic frontier selector `{selector}` for {signature}; expected \
                     a 4-byte hex selector like 0x12345678"
                );
            }
            parsed
        })
        .collect()
}

/// Warns about requested frontier `label`s that the frontier file at `path` did not provide.
fn warn_unimported_frontiers<T: std::fmt::Display + PartialEq>(
    label: &str,
    requested: &[T],
    imported: &[T],
    signature: &str,
    path: &Path,
) {
    for value in requested.iter().filter(|value| !imported.contains(value)) {
        warn!(
            %value,
            label,
            test = %signature,
            path = %path.display(),
            "requested fuzz branch frontier was not imported"
        );
        let _ = sh_warn!(
            "requested fuzz branch frontier {label} {value} was not imported for {signature}"
        );
    }
}

fn frontier_filter_display<T: std::fmt::Display>(values: &[T]) -> String {
    if values.is_empty() { "any".to_string() } else { values.iter().format(", ").to_string() }
}

/// Returns the contract name without the file path prefix.
fn contract_short_name(contract_name: &str) -> &str {
    contract_name.split(':').next_back().unwrap()
}

/// Returns a stable path component that distinguishes overloaded fuzz tests.
fn fuzz_test_path_name<'a>(
    abi: &JsonAbi,
    func: &'a Function,
    config: &FuzzConfig,
    contract_name: &str,
) -> Cow<'a, str> {
    let test_name = format!("{}-{}", func.name, hex::encode(func.selector()));
    let overloaded = abi.functions.get(&func.name).is_some_and(|functions| functions.len() > 1);
    let contract = contract_short_name(contract_name);
    let has_qualified_artifact = config
        .failure_persist_dir
        .as_ref()
        .is_some_and(|dir| dir.join("failures").join(contract).join(&test_name).exists())
        || [&config.corpus.corpus_dir, &config.corpus.frontier_dir]
            .into_iter()
            .flatten()
            .any(|dir| dir.join(contract).join(&test_name).exists());

    if overloaded || has_qualified_artifact {
        Cow::Owned(test_name)
    } else {
        Cow::Borrowed(&func.name)
    }
}

/// Returns whether any canonical replay directory under `dir` holds a corpus entry.
fn corpus_has_entries(dir: &Path) -> bool {
    canonical_replay_dirs(dir).iter().any(|dir| read_corpus_dir(dir).next().is_some())
}

/// Returns the legacy unqualified corpus when the qualified corpus has no entries.
fn legacy_fuzz_corpus_dir(
    root: Option<&Path>,
    contract_name: &str,
    func: &Function,
    test_name: &str,
) -> Option<PathBuf> {
    if test_name == func.name {
        return None;
    }
    let contract = root?.join(contract_short_name(contract_name));
    if corpus_has_entries(&contract.join(test_name)) {
        return None;
    }
    let legacy = contract.join(&func.name);
    corpus_has_entries(&legacy).then(|| canonicalized(legacy))
}

/// Helper function to set test corpus dir and to compose persisted failure paths.
fn test_paths(
    corpus_config: &mut FuzzCorpusConfig,
    persist_dir: PathBuf,
    contract_name: &str,
    test_name: &str,
) -> (PathBuf, PathBuf) {
    let contract = contract_short_name(contract_name);
    // Update config with corpus dir for current test.
    corpus_config.with_test(contract, test_name);

    let failures_dir = canonicalized(persist_dir.join("failures").join(contract));
    let failure_file = canonicalized(failures_dir.join(test_name));
    (failures_dir, failure_file)
}

/// Returns the corpus directory of a contract-level invariant campaign, or of a single
/// optimization campaign.
fn invariant_corpus_dir(
    root: &Path,
    contract_name: &str,
    invariant_name: &str,
    is_optimization: bool,
) -> PathBuf {
    let dir = root.join(contract_short_name(contract_name));
    if is_optimization { dir.join(invariant_name) } else { dir }
}

/// Sets the invariant corpus directory and returns the contract-level failure directory.
fn invariant_suite_paths(
    corpus_config: &mut FuzzCorpusConfig,
    persist_dir: PathBuf,
    contract_name: &str,
    invariant_name: &str,
    is_optimization: bool,
) -> PathBuf {
    if let Some(root) = &corpus_config.corpus_dir {
        corpus_config.corpus_dir = Some(canonicalized(invariant_corpus_dir(
            root,
            contract_name,
            invariant_name,
            is_optimization,
        )));
    }
    canonicalized(persist_dir.join("failures").join(contract_short_name(contract_name)))
}

/// Narrows a generated corpus root to the per-test directory when it exists.
fn narrow_generated_corpus_root(corpus_dir: PathBuf, target_dir: PathBuf) -> PathBuf {
    let target_is_dir =
        std::fs::symlink_metadata(&target_dir).is_ok_and(|metadata| metadata.file_type().is_dir());
    if target_is_dir { canonicalized(target_dir) } else { corpus_dir }
}

fn sanitize_symbolic_artifact_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' })
        .collect::<String>();
    if sanitized.is_empty() { "_".to_string() } else { sanitized }
}

fn symbolic_artifact_file_name(
    contract_id: &str,
    value: &str,
    kind: SymbolicCounterexampleArtifactKind,
) -> String {
    let identity = format!("{contract_id}\0{value}\0{kind:?}");
    let hash = keccak256(identity.as_bytes());
    let hash = hex::encode(&hash[..16]);
    format!("{}-{hash}.json", sanitize_symbolic_artifact_component(value))
}

/// Persists an invariant failure, with any symbolic replay storage and confirmed failure site.
fn record_invariant_failure(
    failure_file: &Path,
    call_sequence: &[BaseCounterExample],
    settings: &InvariantSettings,
    assertion_failure: bool,
    storage: &[SymbolicStorageAssignment],
    failure_site: Option<SymbolicInvariantFailureSite>,
) {
    if let Some(parent) = failure_file.parent()
        && let Err(err) = foundry_common::fs::create_dir_all(parent)
    {
        error!(%err, "Failed to create invariant failure file parent dir");
        return;
    }

    if let Err(err) = foundry_common::fs::write_json_file(
        failure_file,
        &InvariantPersistedFailure {
            call_sequence: call_sequence.to_owned(),
            settings: settings.clone(),
            assertion_failure,
            storage: storage.to_vec(),
            failure_site,
        },
    ) {
        error!(%err, "Failed to record call sequence");
    }
}

/// Persists a handler-side assertion bug with symbolic replay storage.
fn record_handler_failure(
    failure_dir: &Path,
    reverter: Address,
    selector: Selector,
    fingerprint: B256,
    call_sequence: &[BaseCounterExample],
    settings: &InvariantSettings,
    storage: &[SymbolicStorageAssignment],
) {
    let mut buf = [0u8; 24];
    buf[..20].copy_from_slice(reverter.as_slice());
    buf[20..].copy_from_slice(selector.as_slice());
    let file = failure_dir.join("handlers").join(format!("{:x}.json", keccak256(buf)));
    record_invariant_failure(
        &file,
        call_sequence,
        settings,
        true,
        storage,
        Some(SymbolicInvariantFailureSite::SequenceCall {
            target: reverter,
            selector,
            fingerprint,
        }),
    );
}

fn invariant_handler_failure_name(
    identified_contracts: &ContractsByAddress,
    reverter: Address,
    selector: Selector,
) -> String {
    identified_contracts
        .get(&reverter)
        .and_then(|(contract_name, abi)| {
            abi.functions()
                .find(|f| f.selector() == selector)
                .map(|f| format!("{contract_name}::{}", f.name))
        })
        .unwrap_or_else(|| format!("{reverter}::{selector}"))
}

fn should_symbolically_import_fuzz_corpus(config: &Config, func: &Function) -> bool {
    config.symbolic.use_fuzz_corpus && func.test_function_kind().is_fuzz_test()
}

pub(crate) fn effective_test_function_kind(
    kind: TestFunctionKind,
    config: &Config,
    func: &Function,
) -> TestFunctionKind {
    if should_symbolically_import_fuzz_corpus(config, func) {
        TestFunctionKind::SymbolicTest
    } else {
        kind
    }
}

fn symbolic_invariant_unsupported_domain_reason(
    invariant_config: &InvariantConfig,
    sender_filters: &SenderFilters,
    targets: &FuzzRunIdentifiedContracts,
    symbolic_targets: &[SymbolicInvariantTarget],
) -> Option<&'static str> {
    if sender_filters.targeted.is_empty() {
        return Some("symbolic invariant execution requires explicit target senders");
    }
    if invariant_config.has_delay() {
        return Some("symbolic invariant execution does not model warp/roll delays");
    }
    if invariant_config.call_override {
        return Some("symbolic invariant execution does not model call override targets");
    }
    if targets.is_updatable {
        return Some("symbolic invariant execution does not model dynamically updatable targets");
    }
    if invariant_config.corpus.payable_value_weight > 0
        && symbolic_targets
            .iter()
            .any(|target| target.function.state_mutability == StateMutability::Payable)
    {
        return Some("symbolic invariant execution does not model payable call values");
    }
    None
}

/// Replays one corpus-minimization candidate and records its coverage observation.
fn replay_fuzz_minimize<FEN: FoundryEvmNetwork>(
    result: &mut TestResult,
    minimize: &FuzzMinimizeConfig,
    target: String,
    executor: &Executor<FEN>,
    corpus: &FuzzCorpusConfig,
    replay_target: ShowmapReplayTarget<'_>,
) {
    let Ok(mut evm_edge_indices_by_target) = minimize.evm_edge_indices.lock() else {
        result.single_fail(Some("minimize edge index lock poisoned".to_string()));
        return;
    };
    let evm_edge_indices = evm_edge_indices_by_target
        .entry(target.clone())
        .or_insert_with(|| Arc::new(Mutex::new(Default::default())))
        .clone();
    drop(evm_edge_indices_by_target);
    let Ok(mut evm_edge_indices) = evm_edge_indices.lock() else {
        result.single_fail(Some("minimize edge index lock poisoned".to_string()));
        return;
    };
    match replay_sequence_for_minimization(
        executor,
        MinimizationReplayInput {
            sequence: minimize.input.as_ref(),
            evm_edge_indices: &mut evm_edge_indices,
            corpus,
            stop_at_campaign_end: matches!(minimize.mode, FuzzMinimizeMode::Tmin),
        },
        replay_target,
    ) {
        Ok(observation) => {
            let replayed = observation.replayed;
            let skipped = observation.skipped + observation.unmatched;
            let Ok(mut observations) = minimize.observations.lock() else {
                result.single_fail(Some("minimize observations lock poisoned".to_string()));
                return;
            };
            observations.push(FuzzMinimizeObservation { target, observation });
            result.replay_result(replayed, 0, skipped, std::time::Duration::ZERO);
        }
        Err(e) => result.single_fail(Some(e.to_string())),
    }
}
