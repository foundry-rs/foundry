//! The Forge test runner.

use crate::{
    MultiContractRunner, TestFilter,
    coverage::HitMaps,
    fuzz::{BaseCounterExample, FuzzTestResult},
    multi_runner::{TestContract, TestRunnerConfig, symbolic_entrypoints_enabled},
    progress::{TestsProgress, start_fuzz_progress},
    result::{
        InvariantFailure, InvariantPredicateResult, SuiteResult, SymbolicArtifactRef,
        SymbolicCallTrace, SymbolicCounterexample, SymbolicCounterexampleArtifact,
        SymbolicCounterexampleArtifactKind, SymbolicCounterexampleCall,
        SymbolicCounterexampleMinimization, SymbolicCounterexampleReplaySemantics,
        SymbolicCounterexampleTestIdentity, SymbolicReplayMetadata, SymbolicReplayStatus,
        SymbolicResult, TestResult, TestSetup, TestStatus, invariant_campaign_display_name,
    },
    symbolic_minimizer::minimize_single_call_counterexample,
};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, Selector, U256, address, hex, keccak256, map::HashMap};
use eyre::Result;
use foundry_common::{TestFunctionExt, TestFunctionKind, contracts::ContractsByAddress};
use foundry_compilers::utils::canonicalized;
use foundry_config::{Config, FuzzCorpusConfig, InlineConfig, InvariantConfig};
use foundry_evm::{
    constants::{CALLER, CHEATCODE_ADDRESS},
    core::evm::FoundryEvmNetwork,
    decode::{RevertDecoder, SkipReason},
    executors::{
        CallResult, EvmError, Executor, ITest, RawCallResult, ShowmapOpts,
        fuzz::FuzzedExecutor,
        invariant::{
            CheckSequenceOptions, HandlerAssertionFailure, InvariantExecutor, InvariantFuzzError,
            check_sequence, execute_tx, execute_tx_and_register_created, replay_error,
            replay_handler_failure_sequence, replay_run,
        },
        replay_corpus_to_showmap,
    },
    fuzz::{
        BasicTxDetails, CallDetails, CounterExample, FuzzFixtures, fixture_name,
        invariant::{InvariantContract, InvariantSettings, is_optimization_invariant},
        strategies::EvmFuzzState,
    },
    revm::primitives::hardfork::SpecId,
    traces::{TraceKind, TraceMode, load_contracts},
};
use foundry_evm_networks::NetworkVariant;
use foundry_evm_symbolic::{
    SymbolicExecutor, SymbolicRunInput, SymbolicRunResult, SymbolicStats, SymbolicStopReason,
};
use itertools::Itertools;
use proptest::test_runner::{RngAlgorithm, TestError, TestRng, TestRunner};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cmp::min,
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tokio::signal;
use tracing::Span;

/// When running tests, we deploy all external libraries present in the project. To avoid additional
/// libraries affecting nonces of senders used in tests, we are using separate address to
/// predeploy libraries.
///
/// `address(uint160(uint256(keccak256("foundry library deployer"))))`
pub const LIBRARY_DEPLOYER: Address = address!("0x1F95D37F27EA0dEA9C252FC09D5A6eaA97647353");

pub(crate) fn is_symbolic_entrypoint(func: &Function) -> bool {
    func.name.starts_with("check") || func.name.starts_with("prove")
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

fn function_matches_network_pass(
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

fn inline_config_for(
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

        let mut result = TestSetup::default();
        for code in &self.mcr.libs_to_deploy {
            let deploy_result = self.executor.deploy(
                LIBRARY_DEPLOYER,
                code.clone(),
                U256::ZERO,
                Some(&self.mcr.revert_decoder),
            );

            // Record deployed library address.
            if let Ok(deployed) = &deploy_result {
                result.deployed_libs.push(deployed.address);
            }

            let (raw, reason) = RawCallResult::from_evm_result(deploy_result.map(Into::into))?;
            result.extend(raw, TraceKind::Deployment);
            if reason.is_some() {
                debug!(?reason, "deployment of library failed");
                result.reason = reason;
                return Ok(result);
            }
        }

        let address = self.sender.create(self.executor.get_nonce(self.sender)?);
        result.address = address;

        // Set the contracts initial balance before deployment, so it is available during
        // construction
        self.executor.set_balance(address, self.initial_balance())?;

        // Deploy the test contract
        let deploy_result = self.executor.deploy(
            self.sender,
            self.contract.bytecode.clone(),
            U256::ZERO,
            Some(&self.mcr.revert_decoder),
        );

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

        self.executor.deploy_create2_deployer()?;

        // Optionally call the `setUp` function
        if call_setup {
            trace!("calling setUp");
            let res = self.executor.setup(None, address, Some(&self.mcr.revert_decoder));
            let (raw, reason) = RawCallResult::from_evm_result(res)?;
            result.extend(raw, TraceKind::Setup);
            result.reason = reason;
        }

        result.fuzz_fixtures = self.fuzz_fixtures(address);

        Ok(result)
    }

    fn initial_balance(&self) -> U256 {
        self.evm_opts.initial_balance
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
        inline_config_for(&self.config, &self.mcr.inline_config, self.name, func)
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
        FuzzFixtures::new(fixtures)
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(mut self, filter: &dyn TestFilter) -> SuiteResult {
        let start = Instant::now();
        let mut warnings = Vec::new();

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
            // Trip the global fail-fast flag so sibling parallel suites (notably long-running
            // invariant campaigns) observe `should_stop()` and exit at their next run boundary
            // instead of running to their timeout.
            self.tcfg.early_exit.record_failure();
            return SuiteResult::new(
                start.elapsed(),
                [("setUp()".to_string(), TestResult::fail("multiple setUp functions".to_string()))]
                    .into(),
                warnings,
            );
        }

        // Check if `afterInvariant` function with valid signature declared.
        let after_invariant_fns: Vec<_> =
            self.contract.abi.functions().filter(|func| func.name.is_after_invariant()).collect();
        if after_invariant_fns.len() > 1 {
            // Return a single test result failure if multiple functions declared.
            self.tcfg.early_exit.record_failure();
            return SuiteResult::new(
                start.elapsed(),
                [(
                    "afterInvariant()".to_string(),
                    TestResult::fail("multiple afterInvariant functions".to_string()),
                )]
                .into(),
                warnings,
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

        let invariant_fns: Vec<_> =
            self.contract.abi.functions().filter(|func| func.is_invariant_test()).collect();

        // Validate signatures up front: invariant functions must take no parameters. Without
        // this, parameterized `invariant_*` functions would slip into contract-level campaigns
        // and fail with a confusing "selector not found" / decode error mid-campaign. Reject
        // here with a per-function result so the failure is obvious to the user.
        let invalid_invariants: Vec<_> = invariant_fns
            .iter()
            .filter(|f| !f.inputs.is_empty())
            .map(|f| {
                (
                    f.signature(),
                    TestResult::fail(format!(
                        "invariant `{}` must take no parameters",
                        f.signature()
                    )),
                )
            })
            .collect();
        if !invalid_invariants.is_empty() {
            self.tcfg.early_exit.record_failure();
            return SuiteResult::new(
                start.elapsed(),
                invalid_invariants.into_iter().collect(),
                warnings,
            );
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
            self.executor.set_tracing(TraceMode::Call);
            prev_tracer
        });

        let setup_time = Instant::now();
        let setup = self.setup(call_setup);
        debug!("finished setting up in {:?}", setup_time.elapsed());

        if let Some(prev_tracer) = prev_tracer {
            self.executor.inspector_mut().tracer = prev_tracer;
        }

        if setup.reason.is_some() {
            // The setup failed, so we return a single test result for `setUp`
            let fail_msg = if setup.deployment_failure {
                "constructor()".to_string()
            } else {
                "setUp()".to_string()
            };
            self.tcfg.early_exit.record_failure();
            return SuiteResult::new(
                start.elapsed(),
                [(fail_msg, TestResult::setup_result(setup))].into(),
                warnings,
            );
        }

        // Filter out functions sequentially since it's very fast and there is no need to do it
        // in parallel.
        let find_timer = Instant::now();
        let symbolic_enabled = symbolic_entrypoints_enabled(
            self.config.symbolic.enabled,
            self.mcr.tcfg.symbolic_artifact_replay.as_ref(),
        );
        let functions = self
            .contract
            .abi
            .functions()
            .filter(|func| {
                if symbolic_enabled && is_symbolic_entrypoint(func) {
                    filter.matches_test(&func.signature())
                } else {
                    filter.matches_test_function_in_contract(self.name, func)
                }
            })
            .filter(|func| self.function_matches_network_pass(func))
            .collect::<Vec<_>>();
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
            let replay_functions = functions
                .iter()
                .filter(|func| func.signature() == artifact.test.test)
                .copied()
                .collect::<Vec<_>>();

            if replay_functions.is_empty() {
                if !self.mcr.tcfg.multi_network.all_override_networks.is_empty() {
                    return SuiteResult::new(start.elapsed(), BTreeMap::new(), warnings);
                }
                return SuiteResult::new(
                    start.elapsed(),
                    [(
                        artifact.test.test.clone(),
                        TestResult::fail(format!(
                            "symbolic artifact target `{}` was not found in `{}`",
                            artifact.test.test, artifact.test.contract,
                        )),
                    )]
                    .into(),
                    warnings,
                );
            }
            if replay_functions.len() > 1 {
                return SuiteResult::new(
                    start.elapsed(),
                    [(
                        artifact.test.test.clone(),
                        TestResult::fail(format!(
                            "symbolic artifact target `{}` matched {} functions in `{}`",
                            artifact.test.test,
                            replay_functions.len(),
                            artifact.test.contract,
                        )),
                    )]
                    .into(),
                    warnings,
                );
            }

            let test_results = replay_functions
                .into_iter()
                .map(|func| {
                    let start = Instant::now();
                    let kind = match artifact.kind {
                        SymbolicCounterexampleArtifactKind::SingleCall => {
                            TestFunctionKind::SymbolicTest
                        }
                        SymbolicCounterexampleArtifactKind::Sequence => func.test_function_kind(),
                    };
                    if artifact.kind == SymbolicCounterexampleArtifactKind::Sequence
                        && !kind.is_invariant_test()
                    {
                        return (
                            func.signature(),
                            TestResult::fail(format!(
                                "sequence symbolic artifact must target an invariant test, but matched {} function `{}`",
                                kind.name(),
                                func.signature(),
                            )),
                        );
                    }
                    let invariants =
                        if artifact.kind == SymbolicCounterexampleArtifactKind::Sequence {
                            std::slice::from_ref(&func)
                        } else {
                            &[][..]
                        };
                    let mut res = FunctionRunner::new(&self, &setup).run_symbolic_artifact_replay(
                        func,
                        invariants,
                        call_after_invariant,
                    );
                    res.duration = start.elapsed();
                    debug!(%kind, path = %replay.path.display(), "replayed symbolic artifact");
                    (func.signature(), res)
                })
                .collect::<BTreeMap<_, _>>();

            return SuiteResult::new(start.elapsed(), test_results, warnings);
        }

        let test_fail_functions =
            functions.iter().filter(|func| func.test_function_kind().is_any_test_fail());
        if test_fail_functions.clone().next().is_some() {
            let fail = || {
                TestResult::fail("`testFail*` has been removed. Consider changing to test_Revert[If|When]_Condition and expecting a revert".to_string())
            };
            let test_results = test_fail_functions.map(|func| (func.signature(), fail())).collect();
            self.tcfg.early_exit.record_failure();
            return SuiteResult::new(start.elapsed(), test_results, warnings);
        }

        let early_exit = &self.tcfg.early_exit;

        if self.progress.is_some() {
            let interrupt = early_exit.clone();
            self.tokio_handle.spawn(async move {
                signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
                interrupt.record_ctrl_c();
            });
        }

        let invariant_campaigns = select_invariant_campaigns(
            &invariant_fns,
            &functions,
            &self.config,
            &self.mcr.inline_config,
            self.name,
        );
        let InvariantCampaignSelection {
            matched_boolean_invariant_fns,
            merge_boolean_suite: merge_invariant_suite,
            boolean_suite_anchor: invariant_suite_anchor,
            optimization_anchors: _,
        } = invariant_campaigns;

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
                let kind = if symbolic_enabled && is_symbolic_entrypoint(func) {
                    TestFunctionKind::SymbolicTest
                } else {
                    func.test_function_kind()
                };

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

        let duration = start.elapsed();
        SuiteResult::new(duration, test_results, warnings)
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
            tcfg: match &cr.tcfg {
                Cow::Borrowed(tcfg) => Cow::Borrowed(tcfg),
                Cow::Owned(tcfg) => Cow::Owned(tcfg.clone()),
            },
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

    /// Returns whether verbose symbolic diagnostics should be rendered after progress clears.
    fn should_defer_symbolic_diagnostics(&self) -> bool {
        self.cr.progress.is_some() && self.config.symbolic.dump_smt
    }

    fn persist_symbolic_counterexample_artifact(
        &self,
        test_name: &str,
        artifact_file_name: &str,
        symbolic: &SymbolicResult,
        kind: SymbolicCounterexampleArtifactKind,
        calls: Vec<SymbolicCounterexampleCall>,
    ) -> Option<SymbolicArtifactRef> {
        let dir = self
            .config
            .cache_path
            .join("symbolic")
            .join(sanitize_symbolic_artifact_component(self.cr.name));
        // Use a stable per-test artifact path so the latest counterexample replaces older ones.
        let path = dir.join(symbolic_artifact_file_name(self.cr.name, artifact_file_name, kind));
        let artifact = SymbolicCounterexampleArtifact::new(
            kind,
            SymbolicCounterexampleTestIdentity {
                contract: self.cr.name.to_string(),
                test: test_name.to_string(),
            },
            symbolic,
            SymbolicCounterexampleReplaySemantics {
                fail_on_revert: self.config.invariant.fail_on_revert,
            },
            calls,
        );

        if let Err(err) = foundry_common::fs::create_dir_all(&dir) {
            tracing::error!(%err, path = %dir.display(), "Failed to create symbolic artifact dir");
            return None;
        }
        if let Err(err) = foundry_common::fs::write_json_file(&path, &artifact) {
            tracing::error!(%err, path = %path.display(), "Failed to write symbolic artifact");
            return None;
        }

        Some(SymbolicArtifactRef::new(path))
    }

    fn persist_invariant_sequence_counterexample_artifact(
        &self,
        test_name: &str,
        artifact_file_name: &str,
        call_sequence: &[BaseCounterExample],
    ) -> Option<SymbolicArtifactRef> {
        if call_sequence.is_empty() {
            return None;
        }
        if !self.config.symbolic.enabled {
            return None;
        }

        let symbolic_result = SymbolicResult::incomplete(
            &self.config.symbolic,
            SymbolicStopReason::Error,
            "concrete replay confirmed stateful counterexample",
            SymbolicStats::default(),
            SymbolicReplayMetadata::confirmed(),
            SymbolicCallTrace::none(),
            None,
        );
        let calls = call_sequence
            .iter()
            .map(|counterexample| {
                SymbolicCounterexampleCall::from_base_counterexample(
                    counterexample,
                    CALLER,
                    self.address,
                )
            })
            .collect();

        self.persist_symbolic_counterexample_artifact(
            test_name,
            artifact_file_name,
            &symbolic_result,
            SymbolicCounterexampleArtifactKind::Sequence,
            calls,
        )
    }

    fn symbolic_single_call_preserves_failure(
        &self,
        call: &SymbolicCounterexampleCall,
        expected_reason: Option<&str>,
    ) -> bool {
        self.replay_confirmed_symbolic_single_call(call, expected_reason).is_ok()
    }

    fn replay_confirmed_symbolic_single_call(
        &self,
        call: &SymbolicCounterexampleCall,
        expected_reason: Option<&str>,
    ) -> Result<(RawCallResult<FEN>, Option<String>), String> {
        let Some(expected_reason) = expected_reason else {
            return Err("candidate replay has no stable failure reason to compare".to_string());
        };

        let mut executor = self.clone_executor();
        let raw_call_result = execute_tx(&mut executor, &call.to_basic_tx_details())
            .map_err(|err| err.to_string())?;
        if executor.is_raw_call_success(
            self.address,
            Cow::Borrowed(&raw_call_result.state_changeset),
            &raw_call_result,
            false,
        ) {
            return Err("candidate replay succeeded".to_string());
        }

        let reason = self.symbolic_raw_call_failure_reason(&raw_call_result)?;
        if reason.as_deref() != Some(expected_reason) {
            return Err(format!(
                "candidate replay failed with different reason: expected `{}`, got `{}`",
                expected_reason,
                reason.as_deref().unwrap_or("")
            ));
        }

        Ok((raw_call_result, reason))
    }

    fn symbolic_raw_call_failure_reason(
        &self,
        raw_call_result: &RawCallResult<FEN>,
    ) -> Result<Option<String>, String> {
        if raw_call_result.reverter == Some(CHEATCODE_ADDRESS)
            && let Some(reason) = SkipReason::decode(&raw_call_result.result)
        {
            return Err(format!("vm.skip during concrete replay: {reason}"));
        }

        if raw_call_result.reverted
            || raw_call_result.exit_reason.is_some_and(|reason| !reason.is_ok())
        {
            Ok(Some(
                self.revert_decoder().decode(&raw_call_result.result, raw_call_result.exit_reason),
            ))
        } else {
            Ok(None)
        }
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

        // In showmap replay mode, only fuzz/invariant tests are runnable.
        if self.cr.mcr.tcfg.showmap.is_some()
            && matches!(kind, TestFunctionKind::UnitTest { .. } | TestFunctionKind::TableTest)
        {
            self.result.replay_skip("not runnable in showmap mode");
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
        let (mut raw_call_result, reason) = match self.executor.call(
            self.sender,
            self.address,
            func,
            &[],
            U256::ZERO,
            Some(self.revert_decoder()),
        ) {
            Ok(res) => (res.raw, None),
            Err(EvmError::Execution(err)) => (err.raw, Some(err.reason)),
            Err(EvmError::Skip(reason)) => {
                self.result.single_skip(reason);
                return self.result;
            }
            Err(err) => {
                self.result.single_fail(Some(err.to_string()));
                return self.result;
            }
        };

        let success =
            self.executor.is_raw_call_mut_success(self.address, &mut raw_call_result, false);
        self.result.single_result(success, reason, raw_call_result);
        self.result
    }

    /// Runs a symbolic test and replays any discovered counterexample concretely.
    fn run_symbolic_test(mut self, func: &Function) -> TestResult {
        if self.prepare_test(func).is_err() {
            return self.result;
        }

        let mut symbolic = SymbolicExecutor::new(self.config.symbolic.clone());
        if self.should_defer_symbolic_diagnostics() {
            symbolic.capture_diagnostics();
        }
        let result = symbolic.run(SymbolicRunInput {
            executor: self.executor.as_ref(),
            target: self.address,
            sender: self.sender,
            function: func,
            value: U256::ZERO,
            ffi_enabled: self.config.ffi,
        });
        let portfolio_diagnostics = symbolic.portfolio_diagnostics();
        let symbolic_diagnostics = symbolic.take_diagnostics();

        match result {
            SymbolicRunResult::Safe(stats) => {
                self.result.symbolic_result(
                    TestStatus::Success,
                    None,
                    None,
                    SymbolicResult::pass(&self.config.symbolic, stats),
                );
            }
            SymbolicRunResult::Incomplete { kind, reason, stats } => {
                let display_reason = format!("incomplete symbolic execution ({kind:?}): {reason}");
                self.result.symbolic_result(
                    TestStatus::Failure,
                    Some(display_reason),
                    None,
                    SymbolicResult::incomplete(
                        &self.config.symbolic,
                        kind,
                        reason,
                        stats,
                        SymbolicReplayMetadata::not_required(),
                        SymbolicCallTrace::none(),
                        None,
                    ),
                );
            }
            SymbolicRunResult::Counterexample { args, calldata, stats } => {
                let candidate_counterexample =
                    BaseCounterExample::from_fuzz_call(calldata.clone(), args.clone(), None);
                let symbolic_counterexample =
                    SymbolicCounterexample::from(&candidate_counterexample);
                let (raw_call_result, reason) = match self.executor.call(
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
                        let symbolic_result = SymbolicResult::incomplete(
                            &self.config.symbolic,
                            SymbolicStopReason::Error,
                            "concrete replay skipped the symbolic counterexample",
                            stats,
                            SymbolicReplayMetadata::skipped(replay_reason),
                            SymbolicCallTrace::none(),
                            Some(symbolic_counterexample),
                        );
                        self.result.symbolic_result(
                            TestStatus::Skipped,
                            reason.0,
                            None,
                            symbolic_result,
                        );
                        self.result.symbolic_portfolio_diagnostics = portfolio_diagnostics;
                        self.result.symbolic_diagnostics = symbolic_diagnostics;
                        return self.result;
                    }
                    Err(err) => {
                        let reason = err.to_string();
                        let symbolic_result = SymbolicResult::incomplete(
                            &self.config.symbolic,
                            SymbolicStopReason::Error,
                            reason.clone(),
                            stats,
                            SymbolicReplayMetadata::error(reason.clone()),
                            SymbolicCallTrace::none(),
                            Some(symbolic_counterexample),
                        );
                        self.result.symbolic_result(
                            TestStatus::Failure,
                            Some(reason),
                            None,
                            symbolic_result,
                        );
                        self.result.symbolic_portfolio_diagnostics = portfolio_diagnostics;
                        self.result.symbolic_diagnostics = symbolic_diagnostics;
                        return self.result;
                    }
                };

                let success = self.executor.is_raw_call_success(
                    self.address,
                    Cow::Borrowed(&raw_call_result.state_changeset),
                    &raw_call_result,
                    false,
                );
                let call_trace =
                    SymbolicCallTrace::test_result_traces(raw_call_result.traces.is_some());
                let base_counterexample = BaseCounterExample::from_fuzz_call(
                    calldata,
                    args,
                    raw_call_result.traces.clone(),
                );
                if success {
                    self.result.extend(raw_call_result);
                    let reason = "symbolic counterexample did not replay".to_string();
                    let symbolic_result = SymbolicResult::incomplete(
                        &self.config.symbolic,
                        SymbolicStopReason::Error,
                        reason.clone(),
                        stats,
                        SymbolicReplayMetadata::mismatch(reason.clone()),
                        call_trace,
                        Some(symbolic_counterexample),
                    );
                    let counterexample = CounterExample::Single(base_counterexample);
                    self.result.symbolic_result(
                        TestStatus::Failure,
                        Some(reason),
                        Some(counterexample),
                        symbolic_result,
                    );
                } else {
                    let original_base_counterexample = base_counterexample;
                    let original_call = SymbolicCounterexampleCall::from_base_counterexample(
                        &original_base_counterexample,
                        self.sender,
                        self.address,
                    );
                    let original_symbolic_counterexample =
                        SymbolicCounterexample::from(&original_base_counterexample);
                    let mut final_call = original_call.clone();
                    let mut final_raw_call_result = raw_call_result;
                    let mut final_reason = reason;
                    let mut minimization = None;

                    if final_reason.is_some()
                        && let Some(candidate) = minimize_single_call_counterexample(
                            func,
                            &original_call,
                            self.tcfg.config.invariant.shrink_run_limit as usize,
                            |candidate| {
                                self.symbolic_single_call_preserves_failure(
                                    candidate,
                                    final_reason.as_deref(),
                                )
                            },
                        )
                    {
                        if candidate.changed() {
                            match self.replay_confirmed_symbolic_single_call(
                                &candidate.minimized_call,
                                final_reason.as_deref(),
                            ) {
                                Ok((raw_call_result, reason)) => {
                                    final_call = candidate.minimized_call.clone();
                                    final_raw_call_result = raw_call_result;
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

                    let call_trace = SymbolicCallTrace::test_result_traces(
                        final_raw_call_result.traces.is_some(),
                    );
                    let mut base_counterexample = final_call.to_base_counterexample();
                    base_counterexample.traces = final_raw_call_result.traces.clone();
                    let symbolic_counterexample =
                        SymbolicCounterexample::from(&base_counterexample);
                    self.result.extend(final_raw_call_result);

                    let mut symbolic_result = SymbolicResult::fail_counterexample(
                        &self.config.symbolic,
                        stats,
                        call_trace,
                        symbolic_counterexample,
                    );
                    let signature = func.signature();
                    let minimized_artifact = self.persist_symbolic_counterexample_artifact(
                        &signature,
                        &signature,
                        &symbolic_result,
                        SymbolicCounterexampleArtifactKind::SingleCall,
                        vec![final_call],
                    );
                    if let Some(artifact) = minimized_artifact.clone() {
                        symbolic_result = symbolic_result.with_artifact(artifact);
                    }

                    if let Some(minimization) = minimization {
                        let original_symbolic_result = SymbolicResult::fail_counterexample(
                            &self.config.symbolic,
                            stats,
                            SymbolicCallTrace::none(),
                            original_symbolic_counterexample,
                        );
                        let original_artifact = self.persist_symbolic_counterexample_artifact(
                            &signature,
                            &format!("original__{signature}"),
                            &original_symbolic_result,
                            SymbolicCounterexampleArtifactKind::SingleCall,
                            vec![minimization.original_call.clone()],
                        );
                        if let (Some(original), Some(minimized)) =
                            (original_artifact, minimized_artifact)
                        {
                            symbolic_result = symbolic_result.with_minimization(
                                SymbolicCounterexampleMinimization::new(
                                    original,
                                    minimized,
                                    minimization.attempts,
                                    minimization.accepted,
                                    minimization.original_call.calldata.len(),
                                    minimization.minimized_call.calldata.len(),
                                ),
                            );
                        }
                    }
                    let counterexample = CounterExample::Single(base_counterexample);
                    self.result.symbolic_result(
                        TestStatus::Failure,
                        final_reason,
                        Some(counterexample),
                        symbolic_result,
                    );
                }
            }
        }

        self.result.symbolic_portfolio_diagnostics = portfolio_diagnostics;
        self.result.symbolic_diagnostics = symbolic_diagnostics;
        self.result
    }

    /// Replays a durable symbolic counterexample artifact against this freshly set up test.
    fn run_symbolic_artifact_replay(
        mut self,
        func: &Function,
        invariants: &[&Function],
        call_after_invariant: bool,
    ) -> TestResult {
        let Some(replay) = &self.cr.mcr.tcfg.symbolic_artifact_replay else {
            self.result.single_fail(Some("missing symbolic artifact replay config".to_string()));
            return self.result;
        };
        let artifact = &replay.artifact;
        if let Err(e) = self.apply_function_inline_config(func) {
            self.result.single_fail(Some(e.to_string()));
            return self.result;
        }

        match artifact.kind {
            SymbolicCounterexampleArtifactKind::SingleCall => {
                if artifact.replay.status != SymbolicReplayStatus::Confirmed {
                    self.result.single_fail(Some(format!(
                        "single-call symbolic artifact replay status must be confirmed, got {:?}",
                        artifact.replay.status
                    )));
                    return self.result;
                }
                let Some(call) = artifact.calls.first() else {
                    self.result.single_fail(Some("symbolic artifact has no calls".to_string()));
                    return self.result;
                };
                if artifact.calls.len() != 1 {
                    self.result.single_fail(Some(
                        "single-call symbolic artifact must contain exactly one call".to_string(),
                    ));
                    return self.result;
                }
                // Single-call artifacts are concrete replay inputs: sender, value, warp, and roll
                // are intentionally taken from the artifact. Validation only checks that the call
                // still targets this test function.
                if let Err(err) = validate_single_call_symbolic_replay(func, call, self.address) {
                    self.result.single_fail(Some(err));
                    return self.result;
                }

                if self.prepare_test(func).is_err() {
                    return self.result;
                }

                let mut executor = self.clone_executor();
                match execute_tx(&mut executor, &call.to_basic_tx_details()) {
                    Ok(raw_call_result) => {
                        let replay_success = executor.is_raw_call_success(
                            self.address,
                            Cow::Borrowed(&raw_call_result.state_changeset),
                            &raw_call_result,
                            false,
                        );
                        if replay_success {
                            self.result.single_result(true, None, raw_call_result);
                        } else {
                            match raw_call_result.into_evm_error(Some(self.revert_decoder())) {
                                EvmError::Execution(err) => {
                                    let reason = if err.reason.is_empty() {
                                        artifact.replay.reason.clone()
                                    } else {
                                        Some(err.reason.clone())
                                    };
                                    self.result.single_result(false, reason, err.raw);
                                    self.result.counterexample =
                                        Some(CounterExample::Single(call.to_base_counterexample()));
                                }
                                EvmError::Skip(reason) => self.result.single_skip(reason),
                                err => {
                                    self.result.counterexample =
                                        Some(CounterExample::Single(call.to_base_counterexample()));
                                    self.result.single_fail(Some(err.to_string()));
                                }
                            }
                        }
                    }
                    Err(err) => {
                        self.result.counterexample =
                            Some(CounterExample::Single(call.to_base_counterexample()));
                        self.result.single_fail(Some(err.to_string()));
                    }
                }
            }
            SymbolicCounterexampleArtifactKind::Sequence => {
                let Some(invariant) = invariants.first() else {
                    self.result.single_fail(Some(
                        "sequence symbolic artifact must target an invariant test".to_string(),
                    ));
                    return self.result;
                };
                if artifact.calls.is_empty() {
                    self.result.single_fail(Some("symbolic artifact has no calls".to_string()));
                    return self.result;
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
                    return self.result;
                }
                let (sender_filters, targeted) =
                    match evm.select_contracts_and_senders(self.address) {
                        Ok(selected) => selected,
                        Err(err) => {
                            self.result.invariant_setup_fail(err);
                            return self.result;
                        }
                    };
                {
                    let dynamic_target_ctx = evm.dynamic_target_ctx();
                    let mut validation_executor =
                        targeted.is_updatable.then(|| self.clone_executor());
                    let mut validation_created_contracts = Vec::new();
                    for (idx, tx) in txes.iter().enumerate() {
                        let Some(selector) = tx.call_details.calldata.get(..4) else {
                            self.result.single_fail(Some(format!(
                                "sequence symbolic artifact call {} has calldata shorter than a selector",
                                idx + 1
                            )));
                            return self.result;
                        };
                        if !targeted.targets().can_replay(tx) {
                            self.result.single_fail(Some(format!(
                                "sequence symbolic artifact call {} targets unknown function {} on {}",
                                idx + 1,
                                hex::encode_prefixed(selector),
                                tx.call_details.target
                            )));
                            return self.result;
                        }
                        if (!sender_filters.targeted.is_empty()
                            && !sender_filters.targeted.contains(&tx.sender))
                            || sender_filters.excluded.contains(&tx.sender)
                        {
                            self.result.single_fail(Some(format!(
                                "sequence symbolic artifact call {} uses forbidden sender {}",
                                idx + 1,
                                tx.sender
                            )));
                            return self.result;
                        }
                        if let Some(validation_executor) = validation_executor.as_mut() {
                            match execute_tx_and_register_created(
                                validation_executor,
                                tx,
                                &targeted,
                                &dynamic_target_ctx,
                                &mut validation_created_contracts,
                            ) {
                                Ok(()) => {}
                                Err(err) => {
                                    self.result.single_fail(Some(format!(
                                        "sequence symbolic artifact call {} failed during target validation: {err}",
                                        idx + 1
                                    )));
                                    return self.result;
                                }
                            }
                        }
                    }
                }
                match check_sequence(
                    self.clone_executor(),
                    &txes,
                    (0..txes.len()).collect(),
                    self.setup.address,
                    invariant.selector().to_vec().into(),
                    CheckSequenceOptions {
                        // Artifact replay executes every stored call in order, so each call's
                        // warp/roll delta is applied directly. Accumulation is only needed when a
                        // shrink candidate skips calls and must fold removed delays forward.
                        accumulate_warp_roll: false,
                        fail_on_revert: artifact.replay_semantics.fail_on_revert,
                        expect_assertion_failure: false,
                        call_after_invariant,
                        rd: Some(self.revert_decoder()),
                    },
                ) {
                    Ok((success, replayed_entirely, reason, calls_count, reverts)) => {
                        if success {
                            self.result.invariant_replay_success(calls_count, reverts);
                        } else {
                            self.result.invariant_replay_fail(
                                replayed_entirely,
                                &invariant.signature(),
                                reason.or_else(|| artifact.replay.reason.clone()),
                                calls_count,
                                reverts,
                                calls,
                            );
                        }
                    }
                    Err(err) => {
                        self.result.counterexample =
                            Some(CounterExample::Sequence(calls.len(), calls));
                        self.result.single_fail(Some(err.to_string()));
                    }
                }
            }
        }

        self.result
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

        let progress = start_fuzz_progress(
            self.cr.progress,
            self.cr.name,
            &func.name,
            None,
            fixtures_len as u32,
        );

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
            let (mut raw_call_result, reason) = match self.executor.call(
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
                    self.result.single_skip(reason);
                    return self.result;
                }
                Err(err) => {
                    self.result.single_fail(Some(err.to_string()));
                    return self.result;
                }
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
                result.traces = raw_call_result.traces;
                result.debug_bytecodes = raw_call_result.debug_bytecodes;
                self.result.table_result(result);
                return self.result;
            }

            // If it's the last iteration and all other runs succeeded, then use last call result
            // for logs and traces.
            if i == fixtures_len - 1 {
                result.success = true;
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
        let runner = self.invariant_runner();
        let invariant_config = self.config.invariant.clone();
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
        let campaign_anchor = live_invariants
            .iter()
            .find(|(invariant_fn, _)| *invariant_fn == func)
            .map(|(invariant_fn, _)| *invariant_fn)
            .unwrap_or_else(|| live_invariants[0].0);

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
            func.name.as_str(),
            is_optimization,
        );
        // Snapshot the per-test corpus dir before `config` is moved into `InvariantExecutor`.
        let resolved_corpus_dir = config.corpus.corpus_dir.clone();

        let mut evm = InvariantExecutor::new_with_fuzz_seed(
            executor,
            runner,
            self.config.fuzz.seed,
            config,
            identified_contracts,
            &self.cr.mcr.known_contracts,
            self.cr.num_invariant_campaign_anchors,
        );

        // Showmap replay mode: replay the persisted corpus and emit coverage
        // files instead of running the invariant campaign.
        if let Some(showmap) = self.cr.mcr.tcfg.showmap.clone() {
            let corpus_dir = showmap.corpus_dir.clone().or(resolved_corpus_dir);

            // Reconstruct the per-test target selection that the campaign loop normally builds.
            if let Err(e) = evm.select_contract_artifacts(self.address) {
                self.result.invariant_setup_fail(e);
                return self.result;
            }
            let targeted = match evm.select_contracts_and_senders(self.address) {
                Ok((_, t)) => t,
                Err(e) => {
                    self.result.invariant_setup_fail(e);
                    return self.result;
                }
            };
            let dynamic = evm.dynamic_target_ctx();
            return self.run_showmap(
                func,
                corpus_dir,
                &showmap,
                None,
                Some(&targeted),
                Some(&dynamic),
            );
        }

        // Compute current invariant settings up front so secondary persisted-failure handling
        // can use the same compatibility check as the primary replay path below.
        let current_settings = match evm.compute_settings(self.address) {
            Ok(s) => s,
            Err(e) => {
                self.result.invariant_setup_fail(e);
                return self.result;
            }
        };
        // A non-anchor predicate's persisted failure is only honored when its embedded settings
        // still match the current run; stale caches fall back to a fresh campaign.
        let persisted_invariants = if is_optimization {
            BTreeSet::new()
        } else {
            live_invariants
                .iter()
                .filter(|(invariant_fn, _)| *invariant_fn != campaign_anchor)
                .filter_map(|(invariant_fn, _)| {
                    persisted_invariant_failure(&failure_dir, invariant_fn, &current_settings)
                        .is_some()
                        .then_some(invariant_fn.name.as_str())
                })
                .collect::<BTreeSet<_>>()
        };
        // Warn when predicates are dropped because they already have persisted failures from a
        // previous campaign. Symmetric with the primary's persisted-replay warning so users
        // aren't surprised when fewer invariants appear in the report than their contract
        // defines (Echidna/Medusa never skip properties between runs).
        if !is_optimization {
            let persisted_skipped: Vec<&str> = live_invariants
                .iter()
                .filter(|(invariant_fn, _)| {
                    *invariant_fn != campaign_anchor
                        && persisted_invariants.contains(invariant_fn.name.as_str())
                })
                .map(|(invariant_fn, _)| invariant_fn.name.as_str())
                .collect();
            if !persisted_skipped.is_empty() {
                let _ = sh_warn!(
                    "{}: {} invariant(s) skipped due to persisted failures: {}. \
                     Run `forge clean` or delete files in {} to re-include.",
                    self.cr.name,
                    persisted_skipped.len(),
                    persisted_skipped.join(", "),
                    failure_dir.display(),
                );
            }
        }
        // Build the invariant list in source declaration order, retaining the anchor (`func`)
        // and every other selected predicate that doesn't already have a compatible persisted
        // failure. Track the anchor's index so downstream consumers can resolve the campaign
        // anchor without searching by name.
        let invariant_fns: Vec<(&Function, bool)> = live_invariants
            .into_iter()
            .filter(|(invariant_fn, _)| {
                *invariant_fn == campaign_anchor
                    || (!is_optimization
                        && !persisted_invariants.contains(invariant_fn.name.as_str()))
            })
            .collect();
        let anchor_idx = invariant_fns
            .iter()
            .position(|(invariant_fn, _)| *invariant_fn == campaign_anchor)
            .expect("campaign anchor must be present in invariant_fns");
        let predicate_count = invariant_fns.len() + skipped_predicate_results.len();
        let invariant_contract = InvariantContract::new(
            self.address,
            self.cr.name,
            invariant_fns,
            anchor_idx,
            call_after_invariant,
            &self.cr.contract.abi,
        );
        let show_solidity = invariant_config.show_solidity;
        let is_campaign = predicate_count > 1;
        let invariant_count = is_campaign.then_some(predicate_count);
        let invariant_display_name = if is_campaign {
            Cow::Owned(invariant_campaign_display_name(self.cr.name))
        } else {
            Cow::Borrowed(func.name.as_str())
        };

        let progress = start_fuzz_progress(
            self.cr.progress,
            self.cr.name,
            invariant_display_name.as_ref(),
            invariant_config.timeout,
            invariant_config.runs,
        );

        let replay_ctx = ReplayContext {
            invariant_contract: &invariant_contract,
            invariant_config,
            revert_decoder: self.revert_decoder(),
            show_solidity,
        };

        // Try to replay recorded failure if any.
        let primary_failure_file =
            invariant_failure_file(&failure_dir, invariant_contract.anchor());
        let persisted_primary = persisted_invariant_failure(
            &failure_dir,
            invariant_contract.anchor(),
            &current_settings,
        );
        if let Some(InvariantPersistedFailure { mut call_sequence, assertion_failure, .. }) =
            persisted_primary
        {
            let (txes, replay) = replay_persisted_call_sequence(
                &replay_ctx,
                self.clone_executor(),
                &mut call_sequence,
                assertion_failure,
            );
            if let Ok((
                success,
                mut replayed_entirely,
                mut replay_reason,
                mut calls_count,
                mut reverts,
            )) = replay
                && !success
            {
                let warn =
                    "Replayed invariant failure from persisted file. \nRun `forge clean` or remove file to ignore failure and to continue invariant test campaign."
                        .to_string();

                if let Some(ref progress) = progress {
                    progress.set_prefix(format!("{invariant_display_name}\n{warn}\n"));
                } else {
                    let _ = sh_warn!("{warn}");
                }

                // If sequence still fails then replay error to collect traces and exit without
                // executing new runs.
                match replay_error(
                    evm.config(),
                    self.clone_executor(),
                    &txes,
                    None,
                    assertion_failure,
                    None, // check mode
                    &invariant_contract,
                    invariant_contract.anchor(),
                    &self.cr.mcr.known_contracts,
                    identified_contracts.clone(),
                    &mut self.result.logs,
                    &mut self.result.traces,
                    &mut self.result.debug_bytecodes,
                    &mut self.result.line_coverage,
                    &mut self.result.deprecated_cheatcodes,
                    progress.as_ref(),
                    &self.tcfg.early_exit,
                    None, // single-invariant replay path; no [i/N] counter
                ) {
                    Ok(replayed_call_sequence) if !replayed_call_sequence.is_empty() => {
                        call_sequence = replayed_call_sequence;
                        let (_txes, replay) = replay_persisted_call_sequence(
                            &replay_ctx,
                            self.clone_executor(),
                            &mut call_sequence,
                            assertion_failure,
                        );
                        if let Ok((
                            _success,
                            updated_replayed_entirely,
                            updated_replay_reason,
                            updated_calls_count,
                            updated_reverts,
                        )) = replay
                        {
                            replayed_entirely = updated_replayed_entirely;
                            replay_reason = updated_replay_reason;
                            calls_count = updated_calls_count;
                            reverts = updated_reverts;
                        }
                        // Persist error in invariant failure dir.
                        record_invariant_failure(
                            failure_dir.as_path(),
                            primary_failure_file.as_path(),
                            &call_sequence,
                            &current_settings,
                            assertion_failure,
                        );
                    }
                    Ok(_) => {}
                    Err(err) => {
                        error!(%err, "Failed to replay invariant error");
                    }
                }

                self.result.invariant_replay_fail(
                    replayed_entirely,
                    &invariant_contract.anchor().name,
                    replay_reason,
                    calls_count,
                    reverts,
                    call_sequence.clone(),
                );
                if let Some(artifact) = self.persist_invariant_sequence_counterexample_artifact(
                    &invariant_contract.anchor().signature(),
                    &format!("{}-replay", invariant_contract.anchor().signature()),
                    &call_sequence,
                ) {
                    self.result.add_counterexample_artifact(artifact);
                }
                return self.result;
            }
        }

        // Replay persisted handler bugs; feed still-reproducing ones into the campaign,
        // delete stale files in place.
        let persisted_handler_failures = replay_persisted_handler_failures(
            &failure_dir.join("handlers"),
            &current_settings,
            self.clone_executor(),
            &replay_ctx,
        );

        let invariant_result = match evm.invariant_fuzz(
            invariant_contract.clone(),
            &self.setup.fuzz_fixtures,
            self.build_fuzz_state(true),
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
        // Merge coverage collected during invariant run with test setup coverage.
        self.result.merge_coverages(invariant_result.line_coverage);

        let mut counterexample = None;
        // Success requires zero predicate breaks *and* zero handler-side assertion bugs.
        let success =
            invariant_result.errors.is_empty() && invariant_result.handler_errors.is_empty();
        let mut invariant_failures: Vec<InvariantFailure> = vec![];
        let mut any_failure_persisted = false;

        if success {
            if let Some(best_value) = invariant_result.optimization_best_value {
                // Optimization mode: replay and shrink to find shortest best sequence.
                match replay_error(
                    evm.config(),
                    self.clone_executor(),
                    &invariant_result.optimization_best_sequence,
                    None,
                    false,
                    Some(best_value),
                    &invariant_contract,
                    invariant_contract.anchor(),
                    &self.cr.mcr.known_contracts,
                    identified_contracts.clone(),
                    &mut self.result.logs,
                    &mut self.result.traces,
                    &mut self.result.debug_bytecodes,
                    &mut self.result.line_coverage,
                    &mut self.result.deprecated_cheatcodes,
                    progress.as_ref(),
                    &self.tcfg.early_exit,
                    None, // optimization mode is single-invariant; no [i/N] counter
                ) {
                    Ok(best_sequence) if !best_sequence.is_empty() => {
                        counterexample = Some(CounterExample::Sequence(
                            invariant_result.optimization_best_sequence.len(),
                            best_sequence,
                        ));
                    }
                    Err(err) => {
                        error!(%err, "Failed to replay optimization best sequence");
                    }
                    _ => {}
                }
            } else {
                // Standard check mode: replay last run for traces.
                if let Err(err) = replay_run(
                    &invariant_contract,
                    invariant_contract.anchor(),
                    self.clone_executor(),
                    &self.cr.mcr.known_contracts,
                    identified_contracts.clone(),
                    &mut self.result.logs,
                    &mut self.result.traces,
                    &mut self.result.debug_bytecodes,
                    &mut self.result.line_coverage,
                    &mut self.result.deprecated_cheatcodes,
                    &invariant_result.last_run_inputs,
                    show_solidity,
                ) {
                    error!(%err, "Failed to replay last invariant run");
                }
            }
        } else {
            // Total broken invariants in this campaign — used to decorate the shrink progress
            // bar with `[i/N]` so users see how many shrinkers are queued behind the current
            // one. `errors` keys cover both the anchor and any broken secondaries.
            let total_broken = invariant_result.errors.len();
            // Replay-and-shrink the anchor's failure first (gets [1/N] on the progress bar),
            // then push it into `invariant_failures` as the first entry. Non-replayable error
            // variants (e.g. `MaxAssumeRejects`) still get an entry — without a counterexample
            // — so the reason is rendered.
            if let Some(error) = invariant_result.errors.get(&invariant_contract.anchor().name) {
                let anchor_counterexample = match error {
                    InvariantFuzzError::BrokenInvariant(case_data)
                    | InvariantFuzzError::Revert(case_data) => {
                        let TestError::Fail(_, ref calls) = case_data.test_error else {
                            unreachable!("FailedInvariantCaseData::new always sets TestError::Fail")
                        };
                        match replay_error(
                            evm.config(),
                            self.clone_executor(),
                            calls,
                            Some(case_data.inner_sequence.clone()),
                            case_data.assertion_failure,
                            None, // check mode
                            &invariant_contract,
                            invariant_contract.anchor(),
                            &self.cr.mcr.known_contracts,
                            identified_contracts.clone(),
                            &mut self.result.logs,
                            &mut self.result.traces,
                            &mut self.result.debug_bytecodes,
                            &mut self.result.line_coverage,
                            &mut self.result.deprecated_cheatcodes,
                            progress.as_ref(),
                            &self.tcfg.early_exit,
                            Some((1, total_broken)),
                        ) {
                            Ok(call_sequence) if !call_sequence.is_empty() => {
                                record_invariant_failure(
                                    failure_dir.as_path(),
                                    primary_failure_file.as_path(),
                                    &call_sequence,
                                    &current_settings,
                                    case_data.assertion_failure,
                                );
                                any_failure_persisted = true;
                                let artifact = self
                                    .persist_invariant_sequence_counterexample_artifact(
                                        &invariant_contract.anchor().signature(),
                                        &invariant_contract.anchor().signature(),
                                        &call_sequence,
                                    );
                                Some(CounterExample::Sequence(calls.len(), call_sequence))
                                    .map(|counterexample| (counterexample, artifact))
                            }
                            Ok(_) => None,
                            Err(err) => {
                                error!(%err, "Failed to replay invariant error");
                                None
                            }
                        }
                    }
                    InvariantFuzzError::MaxAssumeRejects(_) => None,
                    // Handler bugs live in `handler_errors`; defensive None here.
                    InvariantFuzzError::HandlerAssertion(_) => None,
                };
                let (anchor_counterexample, artifact) = match anchor_counterexample {
                    Some((counterexample, artifact)) => (Some(counterexample), artifact),
                    None => (None, None),
                };
                invariant_failures.push(InvariantFailure::Predicate {
                    name: invariant_contract.anchor().name.clone(),
                    reason: error.revert_reason().unwrap_or_default(),
                    counterexample: anchor_counterexample,
                    artifact,
                    persisted_path: primary_failure_file,
                    is_anchor: true,
                });
            }

            // Shrink each broken non-primary invariant in turn so users get a ready-to-debug
            // counterexample for every failure in a single run. Loop is serial; on Ctrl+C we
            // still record every known secondary failure (without shrinking or persisting), so
            // the final report matches what the live progress bar showed.
            //
            // `next_position` tracks where this invariant sits in the broken queue (primary is
            // 1, secondaries follow). Only incremented when a secondary is actually shrunk so
            // the bar's `[i/N]` counter matches user-visible progress.
            let mut next_position = 2usize;
            // Iterate every invariant; skip the anchor (handled in the primary path above).
            for (idx, (invariant, _)) in invariant_contract.invariant_fns.iter().enumerate() {
                if idx == invariant_contract.anchor_idx {
                    continue;
                }

                // Skip invariants whose counterexample is already persisted from a prior run
                // (those were filtered out of the live campaign earlier; `errors` won't contain
                // them, but the dir check is a belt-and-braces safety net). Use the same
                // settings-aware compatibility check as the filter so a stale persisted cache
                // doesn't suppress a freshly-broken secondary.
                let persisted_failure = invariant_failure_file(&failure_dir, invariant);
                if !persisted_invariants.contains(invariant.name.as_str())
                    && let Some(error) = invariant_result.errors.get(&invariant.name)
                    && let InvariantFuzzError::BrokenInvariant(case_data)
                    | InvariantFuzzError::Revert(case_data) = error
                    && let TestError::Fail(_, ref calls) = case_data.test_error
                {
                    let original_seq_len = calls.len();
                    // On Ctrl+C: skip the (potentially long) replay+shrink, but still persist
                    // the un-shrunk sequence so the next run targeting this invariant picks it
                    // up and shrinks from the saved counterexample. The current run's output
                    // still gets a terse `name: reason` line via the no-counterexample path.
                    let secondary_counterexample = if self.tcfg.early_exit.should_stop() {
                        let unshrunk_sequence = calls
                            .iter()
                            .map(|tx| {
                                BaseCounterExample::from_invariant_call(
                                    tx,
                                    identified_contracts,
                                    None,
                                    invariant_config.show_solidity,
                                )
                            })
                            .collect::<Vec<_>>();
                        record_invariant_failure(
                            failure_dir.as_path(),
                            persisted_failure.as_path(),
                            &unshrunk_sequence,
                            &current_settings,
                            case_data.assertion_failure,
                        );
                        any_failure_persisted = true;
                        None
                    } else {
                        let position = next_position;
                        next_position += 1;
                        match replay_error(
                            invariant_config.clone(),
                            self.clone_executor(),
                            calls,
                            Some(case_data.inner_sequence.clone()),
                            case_data.assertion_failure,
                            None, // check mode
                            &invariant_contract,
                            invariant,
                            &self.cr.mcr.known_contracts,
                            identified_contracts.clone(),
                            &mut self.result.logs,
                            &mut self.result.traces,
                            &mut self.result.debug_bytecodes,
                            &mut self.result.line_coverage,
                            &mut self.result.deprecated_cheatcodes,
                            progress.as_ref(),
                            &self.tcfg.early_exit,
                            Some((position, total_broken)),
                        ) {
                            Ok(call_sequence) if !call_sequence.is_empty() => {
                                record_invariant_failure(
                                    failure_dir.as_path(),
                                    persisted_failure.as_path(),
                                    &call_sequence,
                                    &current_settings,
                                    case_data.assertion_failure,
                                );
                                any_failure_persisted = true;
                                let artifact = self
                                    .persist_invariant_sequence_counterexample_artifact(
                                        &invariant.signature(),
                                        &invariant.signature(),
                                        &call_sequence,
                                    );
                                Some((
                                    CounterExample::Sequence(original_seq_len, call_sequence),
                                    artifact,
                                ))
                            }
                            Ok(_) => None,
                            Err(err) => {
                                error!(%err, "Failed to replay invariant error");
                                None
                            }
                        }
                    };
                    let (secondary_counterexample, artifact) = match secondary_counterexample {
                        Some((counterexample, artifact)) => (Some(counterexample), artifact),
                        None => (None, None),
                    };
                    invariant_failures.push(InvariantFailure::Predicate {
                        name: invariant.name.clone(),
                        reason: error.revert_reason().unwrap_or_default(),
                        counterexample: secondary_counterexample,
                        artifact,
                        persisted_path: persisted_failure.clone(),
                        is_anchor: false,
                    });
                }
            }
        }

        let invariant_failure_dir = any_failure_persisted.then(|| failure_dir.clone());
        let invariant_predicate_results = if is_campaign {
            let failures_by_name = invariant_failures
                .iter()
                .map(|failure| (failure.name(), failure))
                .collect::<BTreeMap<_, _>>();
            invariant_contract
                .invariant_fns
                .iter()
                .map(|(invariant, _)| {
                    if let Some(failure) = failures_by_name.get(invariant.name.as_str()) {
                        InvariantPredicateResult {
                            name: invariant.name.clone(),
                            status: TestStatus::Failure,
                            reason: Some(failure.reason().to_string()),
                        }
                    } else {
                        InvariantPredicateResult {
                            name: invariant.name.clone(),
                            status: TestStatus::Success,
                            reason: None,
                        }
                    }
                })
                .chain(skipped_predicate_results)
                .sorted_by_key(|predicate| {
                    self.cr
                        .contract
                        .abi
                        .functions()
                        .position(|func| func.name == predicate.name)
                        .unwrap_or(usize::MAX)
                })
                .collect()
        } else {
            Vec::new()
        };

        // Convert handler-side assertion bugs into render-ready entries. The name is a
        // best-effort `Contract::function` from `identified_contracts`, falling back to
        // `0xreverter::0xselector`. Map is keyed by `(reverter, selector)` site so multiple
        // code paths through the same function collapse to one entry, rendered in the
        // dedicated handler assertions section.
        let identified_contracts_ro = identified_contracts;
        let invariant_handler_failures = invariant_result
            .handler_errors
            .iter()
            .sorted_by(|(ka, _), (kb, _)| {
                // Stable order across runs: sort by `(reverter, selector)` site directly.
                ka.cmp(kb)
            })
            .filter_map(|(site, err)| err.as_handler_assertion().map(|f| (site, f)))
            .map(|(_site, failure)| {
                let reverter = failure.reverter;
                let selector = failure.selector;
                // Resolve `Contract::function` from identified contracts when possible.
                let resolved_name = identified_contracts_ro
                    .get(&reverter)
                    .and_then(|(contract_name, abi)| {
                        abi.functions()
                            .find(|f| f.selector() == selector)
                            .map(|f| format!("{contract_name}::{}", f.name))
                    })
                    .unwrap_or_else(|| format!("{reverter}::{selector}"));

                let counterexample_calls = failure
                    .call_sequence
                    .iter()
                    .map(|tx| {
                        BaseCounterExample::from_invariant_call(
                            tx,
                            identified_contracts_ro,
                            None,
                            invariant_config.show_solidity,
                        )
                    })
                    .collect::<Vec<_>>();

                // Persist for next-run replay (skip if nothing to record).
                if !counterexample_calls.is_empty() {
                    record_handler_failure(
                        failure_dir.as_path(),
                        reverter,
                        selector,
                        &counterexample_calls,
                        &current_settings,
                    );
                }
                let artifact = self.persist_invariant_sequence_counterexample_artifact(
                    &invariant_contract.anchor().signature(),
                    &format!("handler-{reverter}-{selector}"),
                    &counterexample_calls,
                );

                let counterexample = if counterexample_calls.is_empty() {
                    None
                } else {
                    // Preserve pre-shrink length for `(original: N, shrunk: M)` rendering.
                    Some(CounterExample::Sequence(
                        failure.original_sequence_len,
                        counterexample_calls,
                    ))
                };

                InvariantFailure::Handler {
                    name: resolved_name,
                    reverter,
                    selector,
                    reason: failure.revert_reason.clone(),
                    counterexample,
                    artifact,
                }
            })
            .collect::<Vec<_>>();

        self.result.invariant_result(
            invariant_result.gas_report_traces,
            success,
            invariant_failures,
            invariant_predicate_results,
            invariant_failure_dir,
            invariant_count,
            invariant_handler_failures,
            counterexample,
            invariant_result.runs,
            invariant_result.calls,
            invariant_result.reverts,
            invariant_result.metrics,
            invariant_result.failed_corpus_replays,
            invariant_result.workers,
            invariant_result.optimization_best_value,
        );
        self.result
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
        let (failure_dir, failure_file) = test_paths(
            &mut fuzz_config.corpus,
            fuzz_config.failure_persist_dir.clone().unwrap(),
            self.cr.name,
            &func.name,
        );

        // Showmap replay mode: replay the persisted corpus and emit coverage
        // files instead of running the fuzz campaign.
        if let Some(showmap) = self.cr.mcr.tcfg.showmap.clone() {
            let corpus_dir =
                showmap.corpus_dir.clone().or_else(|| fuzz_config.corpus.corpus_dir.clone());
            return self.run_showmap(func, corpus_dir, &showmap, Some(func), None, None);
        }

        let progress = start_fuzz_progress(
            self.cr.progress,
            self.cr.name,
            &func.name,
            fuzz_config.timeout,
            if fuzz_config.run.is_some() { 1 } else { fuzz_config.runs },
        );

        let state = self.build_fuzz_state(false);
        let mut executor = self.executor.into_owned();
        // Enable edge coverage if running with coverage guided fuzzing or with edge coverage
        // metrics (useful for benchmarking the fuzzer).
        executor.inspector_mut().collect_edge_coverage_with_config(&fuzz_config.corpus);
        executor.inspector_mut().collect_evm_cmp_log(fuzz_config.corpus.collect_evm_cmp_log());
        executor.inspector_mut().collect_sancov_edges(fuzz_config.corpus.collect_sancov_edges());
        executor
            .inspector_mut()
            .collect_sancov_trace_cmp(fuzz_config.corpus.collect_sancov_trace_cmp());
        // Load persisted counterexample, if any.
        let persisted_failure =
            foundry_common::fs::read_json_file::<BaseCounterExample>(failure_file.as_path()).ok();
        // Run fuzz test.
        let mut fuzzed_executor =
            FuzzedExecutor::new(executor, runner, self.tcfg.sender, fuzz_config, persisted_failure);
        let result = match fuzzed_executor.fuzz(
            func,
            &self.setup.fuzz_fixtures,
            state,
            self.address,
            &self.cr.mcr.revert_decoder,
            progress.as_ref(),
            &self.tcfg.early_exit,
            &self.cr.tokio_handle,
        ) {
            Ok(x) => x,
            Err(e) => {
                self.result.fuzz_setup_fail(e);
                return self.result;
            }
        };

        // Record counterexample.
        if let Some(CounterExample::Single(counterexample)) = &result.counterexample {
            if let Err(err) = foundry_common::fs::create_dir_all(failure_dir) {
                error!(%err, "Failed to create fuzz failure dir");
            } else if let Err(err) =
                foundry_common::fs::write_json_file(failure_file.as_path(), counterexample)
            {
                error!(%err, "Failed to record call sequence");
            }
        }

        self.result.fuzz_result(result);
        self.result
    }

    /// Prepares single unit test and fuzz test execution:
    /// - set up the test result and executor
    /// - check if before test txes are configured and apply them in order
    ///
    /// Before test txes are arrays of arbitrary calldata obtained by calling the `beforeTest`
    /// function with test selector as a parameter.
    ///
    /// Unit tests within same contract (or even current test) are valid options for before test tx
    /// configuration. Test execution stops if any of before test txes fails.
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
                match self.executor.to_mut().transact_raw(
                    self.tcfg.sender,
                    address,
                    calldata,
                    U256::ZERO,
                ) {
                    Ok(call_result) => {
                        let reverted = call_result.reverted;

                        // Merge tx result traces in unit test result.
                        self.result.extend(call_result);

                        // To continue unit test execution the call should not revert.
                        if reverted {
                            self.result.single_fail(None);
                            return Err(());
                        }
                    }
                    Err(_) => {
                        self.result.single_fail(None);
                        return Err(());
                    }
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
        corpus_dir: Option<PathBuf>,
        showmap: &crate::multi_runner::ShowmapConfig,
        fuzzed_function: Option<&Function>,
        fuzzed_contracts: Option<&foundry_evm::fuzz::invariant::FuzzRunIdentifiedContracts>,
        dynamic: Option<&foundry_evm::executors::DynamicTargetCtx<'_>>,
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
        // trials of a single test — what `differential-coverage` expects. Invariant
        // tests share one corpus per contract, so omit the function name for them
        // to avoid emitting duplicate approach dirs that replay the same corpus.
        let safe_id = self.cr.name.replace(['/', '\\', ':'], "_");
        let approach = if fuzzed_contracts.is_some() {
            format!("{}__{safe_id}", showmap.approach)
        } else {
            let safe_fn = func.name.replace(['/', '\\', ':', '(', ')', ',', ' '], "_");
            format!("{}__{safe_id}__{safe_fn}", showmap.approach)
        };
        let opts = ShowmapOpts {
            out_dir: showmap.out_dir.clone(),
            approach,
            trial: showmap.trial.clone(),
            per_input: showmap.per_input,
            domain,
        };

        let start = std::time::Instant::now();
        let result = replay_corpus_to_showmap(
            &executor,
            &corpus_dir,
            fuzzed_function,
            fuzzed_contracts,
            dynamic,
            &opts,
        );
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
                self.result.replay_result(
                    stats.corpus_entries,
                    stats.showmap_files,
                    stats.skipped_entries,
                    duration,
                );
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

    fn build_fuzz_state(&self, invariant: bool) -> EvmFuzzState {
        let config =
            if invariant { self.config.invariant.dictionary } else { self.config.fuzz.dictionary };
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
}

/// Mirrors `check_sequence`'s return:
/// `(success, replayed_entirely, optional_reason, calls, reverts)`.
type CheckSequenceResult = eyre::Result<(bool, bool, Option<String>, usize, usize)>;

/// Borrowed context shared by primary-invariant and handler-side replay helpers.
struct ReplayContext<'a> {
    invariant_contract: &'a InvariantContract<'a>,
    invariant_config: &'a InvariantConfig,
    revert_decoder: &'a RevertDecoder,
    show_solidity: bool,
}

/// Helper function to load failed call sequence from file.
/// Ignores failure if generated with different invariant settings than the current ones.
fn persisted_call_sequence(
    path: &Path,
    current_settings: &InvariantSettings,
) -> Option<InvariantPersistedFailure> {
    foundry_common::fs::read_json_file::<InvariantPersistedFailure>(path).ok().and_then(
        |persisted_failure| {
            if let Some(diff) = persisted_failure.settings.diff(current_settings) {
                let _ = sh_warn!(
                    "Failure from {:?} file was ignored because invariant test settings have changed: {}",
                    path,
                    diff
                );
                return None;
            }
            Some(persisted_failure)
        },
    )
}

/// Returns the current invariant failure cache path.
fn invariant_failure_file(failure_dir: &Path, invariant: &Function) -> PathBuf {
    canonicalized(failure_dir.join("invariants").join(&invariant.name))
}

/// Returns the legacy invariant failure cache path.
fn legacy_invariant_failure_file(failure_dir: &Path, invariant: &Function) -> PathBuf {
    canonicalized(failure_dir.join(&invariant.name))
}

/// Loads a persisted invariant failure from the new cache path, falling back to the legacy path.
fn persisted_invariant_failure(
    failure_dir: &Path,
    invariant: &Function,
    current_settings: &InvariantSettings,
) -> Option<InvariantPersistedFailure> {
    persisted_call_sequence(invariant_failure_file(failure_dir, invariant).as_path(), current_settings)
        .or_else(|| {
            let legacy_path = legacy_invariant_failure_file(failure_dir, invariant);
            let persisted = persisted_call_sequence(legacy_path.as_path(), current_settings)?;
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
    ctx: &ReplayContext<'_>,
    call_sequence: &mut [BaseCounterExample],
) -> Vec<BasicTxDetails> {
    call_sequence
        .iter_mut()
        .map(|seq| {
            seq.show_solidity = ctx.show_solidity;
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
        })
        .collect()
}

/// Converts a persisted `BaseCounterExample` sequence into `BasicTxDetails` (applying
/// `ctx.show_solidity` in place) and replays it via `check_sequence`.
fn replay_persisted_call_sequence<FEN: FoundryEvmNetwork>(
    ctx: &ReplayContext<'_>,
    executor: Executor<FEN>,
    call_sequence: &mut [BaseCounterExample],
    expect_assertion_failure: bool,
) -> (Vec<BasicTxDetails>, CheckSequenceResult) {
    let txes = base_counterexamples_to_txes(ctx, call_sequence);
    let result = check_sequence(
        executor,
        &txes,
        (0..min(txes.len(), ctx.invariant_config.depth as usize)).collect(),
        ctx.invariant_contract.address,
        ctx.invariant_contract.anchor().selector().to_vec().into(),
        CheckSequenceOptions {
            accumulate_warp_roll: ctx.invariant_config.has_delay(),
            fail_on_revert: ctx.invariant_config.fail_on_revert,
            expect_assertion_failure,
            call_after_invariant: ctx.invariant_contract.call_after_invariant,
            rd: Some(ctx.revert_decoder),
        },
    );
    (txes, result)
}

/// Helper function to set test corpus dir and to compose persisted failure paths.
fn test_paths(
    corpus_config: &mut FuzzCorpusConfig,
    persist_dir: PathBuf,
    contract_name: &str,
    test_name: &str,
) -> (PathBuf, PathBuf) {
    let contract = contract_name.split(':').next_back().unwrap();
    // Update config with corpus dir for current test.
    corpus_config.with_test(contract, test_name);

    let failures_dir = canonicalized(persist_dir.join("failures").join(contract));
    let failure_file = canonicalized(failures_dir.join(test_name));
    (failures_dir, failure_file)
}

/// Sets the invariant corpus directory and returns the contract-level failure directory.
fn invariant_suite_paths(
    corpus_config: &mut FuzzCorpusConfig,
    persist_dir: PathBuf,
    contract_name: &str,
    invariant_name: &str,
    is_optimization: bool,
) -> PathBuf {
    let failure_dir = invariant_failure_dir(persist_dir, contract_name);
    let contract = invariant_contract_name(contract_name);
    if let Some(corpus_dir) = &corpus_config.corpus_dir {
        let mut corpus_dir = corpus_dir.join(contract);
        if is_optimization {
            corpus_dir = corpus_dir.join(invariant_name);
        }
        corpus_config.corpus_dir = Some(canonicalized(corpus_dir));
    }

    failure_dir
}

/// Returns the contract-level invariant failure directory.
fn invariant_failure_dir(persist_dir: PathBuf, contract_name: &str) -> PathBuf {
    canonicalized(persist_dir.join("failures").join(invariant_contract_name(contract_name)))
}

/// Returns the invariant test contract name without the file path prefix.
fn invariant_contract_name(contract_name: &str) -> &str {
    contract_name.split(':').next_back().unwrap()
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

fn validate_single_call_symbolic_replay(
    func: &Function,
    call: &SymbolicCounterexampleCall,
    test_address: Address,
) -> Result<(), String> {
    if call.target != test_address {
        return Err(format!(
            "single-call symbolic artifact target {} does not match test contract {}",
            call.target, test_address
        ));
    }
    if call.calldata.get(..4).is_none_or(|selector| func.selector() != selector) {
        return Err(format!(
            "single-call symbolic artifact calldata does not match `{}` selector",
            func.signature()
        ));
    }
    Ok(())
}

/// Helper function to persist invariant failure.
fn record_invariant_failure(
    failure_dir: &Path,
    failure_file: &Path,
    call_sequence: &[BaseCounterExample],
    settings: &InvariantSettings,
    assertion_failure: bool,
) {
    if let Err(err) = foundry_common::fs::create_dir_all(failure_dir) {
        error!(%err, "Failed to create invariant failure dir");
        return;
    }
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
        },
    ) {
        error!(%err, "Failed to record call sequence");
    }
}

/// Persists a handler-side assertion bug under `<failure_dir>/handlers/<site>.json`,
/// where `<site>` is `keccak256(reverter || selector)`.
fn record_handler_failure(
    failure_dir: &Path,
    reverter: Address,
    selector: Selector,
    call_sequence: &[BaseCounterExample],
    settings: &InvariantSettings,
) {
    let handlers_dir = failure_dir.join("handlers");
    if let Err(err) = foundry_common::fs::create_dir_all(&handlers_dir) {
        error!(%err, "Failed to create handler failure dir");
        return;
    }
    let mut buf = [0u8; 24];
    buf[..20].copy_from_slice(reverter.as_slice());
    buf[20..].copy_from_slice(selector.as_slice());
    let site_hash = alloy_primitives::keccak256(buf);
    let file = handlers_dir.join(format!("{site_hash:x}.json"));
    record_invariant_failure(&handlers_dir, &file, call_sequence, settings, true);
}

/// Replays persisted handler-side assertion bugs. A file is kept only if the anchor still
/// asserts at the same `(reverter, selector)` site; stale files (anchor no longer asserts,
/// asserts at a different site, or earlier call asserts) are deleted in place.
fn replay_persisted_handler_failures<FEN: FoundryEvmNetwork>(
    handlers_dir: &Path,
    current_settings: &InvariantSettings,
    executor: Executor<FEN>,
    ctx: &ReplayContext<'_>,
) -> std::collections::HashMap<(Address, Selector), InvariantFuzzError> {
    let mut replayed: std::collections::HashMap<(Address, Selector), InvariantFuzzError> =
        std::collections::HashMap::new();
    let entries = match std::fs::read_dir(handlers_dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return replayed,
        Err(err) => {
            error!(%err, "Failed to read handler failure dir");
            return replayed;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(persisted) = persisted_call_sequence(&path, current_settings) else {
            continue;
        };
        let mut call_sequence = persisted.call_sequence;
        if call_sequence.is_empty() {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let txes = base_counterexamples_to_txes(ctx, &mut call_sequence);
        // Expected site = (target, selector) of the persisted reproducer's last call.
        let Some(last) = txes.last() else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let expected_target = last.call_details.target;
        let expected_selector_bytes: [u8; 4] =
            last.call_details.calldata.get(..4).and_then(|s| s.try_into().ok()).unwrap_or_default();
        let expected_site = (expected_target, Selector::from(expected_selector_bytes));
        let sequence: Vec<usize> =
            (0..min(txes.len(), ctx.invariant_config.depth as usize)).collect();
        let outcome = replay_handler_failure_sequence(
            executor.clone(),
            &txes,
            sequence,
            ctx.invariant_config.has_delay(),
            Some(ctx.revert_decoder),
        );
        match outcome {
            Ok(outcome) if outcome.anchor_asserted => {
                let _ = sh_warn!(
                    "Replayed handler-side assertion bug from {path:?}. \nRun `forge clean` or remove file to ignore."
                );
                let failure = HandlerAssertionFailure::from_replayed_sequence(
                    txes,
                    outcome.anchor_fingerprint,
                    outcome.revert_reason.unwrap_or_default(),
                );
                // On collision keep the shorter reproducer. Inlined: `replayed` uses the legacy
                // `(reverter, selector)` key, not the unified `FailureKey`.
                let already_shorter = replayed
                    .get(&expected_site)
                    .and_then(InvariantFuzzError::as_handler_assertion)
                    .is_some_and(|existing| {
                        existing.call_sequence.len() <= failure.call_sequence.len()
                    });
                if !already_shorter {
                    replayed.insert(expected_site, InvariantFuzzError::HandlerAssertion(failure));
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
    replayed
}
