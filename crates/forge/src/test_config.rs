//! Execution-independent test configuration and fuzz replay paths.

use crate::{
    TestFilter,
    result::{SymbolicCounterexampleArtifact, SymbolicCounterexampleArtifactKind},
};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::U256;
use eyre::Result;
use foundry_common::{
    TestFunctionExt, TestFunctionKind, external_compiler::external_artifact_is_test_eligible,
};
use foundry_compilers::ArtifactId;
use foundry_config::{Config, FuzzConfig, InlineConfig};
use foundry_evm_fuzz::invariant::is_optimization_invariant;
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};
use std::borrow::Cow;

#[cfg(feature = "revm")]
use std::path::PathBuf;

pub(crate) const SYMBOLIC_REGRESSION_MARKER: &str = "__foundry_symbolic_regression_marker";

pub(crate) fn inline_config_for(
    config: &Config,
    inline_config: &InlineConfig,
    contract_name: &str,
    func: Option<&Function>,
) -> Result<Config> {
    let function = func.map(|f| f.name.as_str()).unwrap_or("");
    Ok(config.merge_inline_provider(inline_config.provide(contract_name, function))?)
}

pub(crate) fn fuzzer_with_cases(
    seed: Option<U256>,
    cases: u32,
    max_global_rejects: u32,
) -> TestRunner {
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

/// Returns the contract name without the file path prefix.
pub(crate) fn contract_short_name(contract_name: &str) -> &str {
    contract_name.split(':').next_back().unwrap()
}

/// Returns a stable path component that distinguishes overloaded fuzz tests.
pub(crate) fn fuzz_test_path_name<'a>(
    abi: &JsonAbi,
    func: &'a Function,
    config: &FuzzConfig,
    contract_name: &str,
) -> Cow<'a, str> {
    let test_name = format!("{}-{:x}", func.name, func.selector());
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

#[derive(Clone, Debug)]
pub struct SymbolicArtifactReplayConfig {
    /// Artifact payload to replay.
    pub artifact: SymbolicCounterexampleArtifact,
    /// Path the artifact was loaded from, used in diagnostics.
    #[cfg(feature = "revm")]
    pub path: PathBuf,
}

#[derive(Clone, Copy)]
pub(crate) struct TestFunctionMatcher<'a> {
    pub(crate) config: &'a Config,
    pub(crate) inline_config: &'a InlineConfig,
    symbolic_artifact_replay: Option<&'a SymbolicArtifactReplayConfig>,
}

impl<'a> TestFunctionMatcher<'a> {
    pub(crate) const fn new(
        config: &'a Config,
        inline_config: &'a InlineConfig,
        symbolic_artifact_replay: Option<&'a SymbolicArtifactReplayConfig>,
    ) -> Self {
        Self { config, inline_config, symbolic_artifact_replay }
    }

    fn symbolic_tests_enabled(&self, contract_id: &str) -> bool {
        self.symbolic_artifact_replay.is_some_and(|artifact| {
            artifact.artifact.kind == SymbolicCounterexampleArtifactKind::SingleCall
        }) || self.inline_config.contract_symbolic_enabled(
            &self.config.profile,
            contract_id,
            self.config.symbolic.enabled,
        )
    }

    pub(crate) fn test_function_kind(
        &self,
        contract_id: &str,
        func: &Function,
        generated_symbolic_regression: bool,
    ) -> TestFunctionKind {
        if generated_symbolic_regression && !func.name.starts_with("test_regression_") {
            return TestFunctionKind::Unknown;
        }

        TestFunctionKind::classify(
            func.name.as_str(),
            !func.inputs.is_empty(),
            self.symbolic_tests_enabled(contract_id),
        )
    }

    /// Returns the functions of `abi` accepted by `keep`, which is given the contract identifier,
    /// the function and its classification.
    pub(crate) fn test_functions(
        self,
        contract_id: String,
        abi: &JsonAbi,
        mut keep: impl FnMut(&str, &Function, TestFunctionKind) -> bool,
    ) -> impl Iterator<Item = &Function> {
        let generated_symbolic_regression = is_generated_symbolic_regression_contract(abi);
        abi.functions().filter(move |func| {
            let kind = self.test_function_kind(&contract_id, func, generated_symbolic_regression);
            keep(&contract_id, func, kind)
        })
    }

    /// Returns the test functions of `abi` that match `filter`.
    pub(crate) fn matching_test_functions<'b>(
        self,
        filter: &dyn TestFilter,
        id: &ArtifactId,
        abi: &'b JsonAbi,
    ) -> impl Iterator<Item = &'b Function> {
        self.test_functions(id.identifier(), abi, move |contract_id, func, kind| {
            filter.matches_test_function_kind_in_contract(contract_id, func, kind)
        })
    }

    pub(crate) fn matches_contract(
        &self,
        filter: &dyn TestFilter,
        id: &ArtifactId,
        abi: &JsonAbi,
    ) -> bool {
        external_artifact_is_test_eligible(&id.build_id)
            && filter.matches_path(&id.source)
            && filter.matches_contract(&id.name)
            && self.matching_test_functions(filter, id, abi).next().is_some()
    }
}

pub(crate) fn is_generated_symbolic_regression_contract(abi: &JsonAbi) -> bool {
    abi.functions().any(|func| func.name == SYMBOLIC_REGRESSION_MARKER && func.inputs.is_empty())
}

pub(crate) fn effective_test_function_kind(
    kind: TestFunctionKind,
    config: &Config,
    func: &Function,
) -> TestFunctionKind {
    if config.symbolic.use_fuzz_corpus && func.test_function_kind().is_fuzz_test() {
        TestFunctionKind::SymbolicTest
    } else {
        kind
    }
}

pub(crate) struct InvariantCampaignSelection<'a> {
    pub(crate) matched_boolean_invariant_fns: Vec<&'a Function>,
    pub(crate) merge_boolean_suite: bool,
    pub(crate) shared_boolean_namespace: bool,
    pub(crate) boolean_suite_anchor: Option<&'a Function>,
    pub(crate) optimization_anchors: usize,
}

impl InvariantCampaignSelection<'_> {
    pub(crate) const fn anchor_count(&self) -> usize {
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

pub(crate) fn select_invariant_campaigns<'a>(
    invariant_fns: &[&'a Function],
    functions: &[&'a Function],
    config: &Config,
    inline_config: &InlineConfig,
    contract_name: &str,
) -> InvariantCampaignSelection<'a> {
    let boolean_invariant_fns = invariant_fns
        .iter()
        .copied()
        .filter(|func| !is_optimization_invariant(func))
        .collect::<Vec<_>>();
    let matched_boolean_invariant_fns = functions
        .iter()
        .copied()
        .filter(|func| func.is_invariant_test() && !is_optimization_invariant(func))
        .collect::<Vec<_>>();
    let optimization_anchors = functions
        .iter()
        .filter(|func| func.is_invariant_test() && is_optimization_invariant(func))
        .count();

    // Merge compatible selected predicates even when an excluded predicate has different config.
    // Decide the corpus/frontier namespace separately from the full suite so filtering cannot
    // move an isolated campaign into the contract-level namespace.
    let canonical_boolean_anchor = boolean_invariant_fns.first().copied();
    let merge_boolean_suite = !matched_boolean_invariant_fns.is_empty()
        && invariant_suite_configs_match(
            config,
            inline_config,
            contract_name,
            &matched_boolean_invariant_fns,
        );
    let shared_boolean_namespace = merge_boolean_suite
        && invariant_suite_configs_match(
            config,
            inline_config,
            contract_name,
            &boolean_invariant_fns,
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
        shared_boolean_namespace,
        boolean_suite_anchor,
        optimization_anchors,
    }
}
