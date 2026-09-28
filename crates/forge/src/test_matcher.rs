//! Engine-independent Forge test selection.

use crate::{
    TestFilter,
    result::{SymbolicCounterexampleArtifact, SymbolicCounterexampleArtifactKind},
    runner::{
        InvariantCampaignScope, count_runnable_invariant_campaign_anchors,
        function_matches_network_pass,
    },
    symbolic_regression::SYMBOLIC_REGRESSION_MARKER,
};
use alloy_json_abi::{Function, JsonAbi};
use foundry_common::{TestFunctionKind, external_compiler::external_artifact_is_test_eligible};
use foundry_compilers::ArtifactId;
use foundry_config::{Config, InlineConfig};
use foundry_evm_networks::NetworkVariant;
use std::path::PathBuf;

/// Tracks network assignment across a multi-network test run.
///
/// When inline config specifies different networks for different tests, the runner performs one
/// pass per distinct network. This struct encodes which pass we're in so each `ContractRunner`
/// can skip tests that belong to a different pass.
///
/// Default (empty `all_override_networks`, `None` pass) = single-pass mode, every test runs.
#[derive(Clone, Debug, Default)]
pub struct MultiNetworkConfig {
    /// All networks explicitly referenced in inline config annotations across the whole suite.
    /// Empty means single-pass mode (no per-test network overrides present).
    pub all_override_networks: Vec<NetworkVariant>,
    /// The network this pass is responsible for.
    /// `None` = default pass: runs tests *without* an explicit network annotation (or annotated
    /// with a network not in `all_override_networks`).
    /// `Some(v)` = override pass: runs only tests annotated with exactly `v`.
    pub pass_network: Option<NetworkVariant>,
}

#[derive(Clone, Debug)]
pub struct SymbolicArtifactReplayConfig {
    /// Artifact payload to replay.
    pub artifact: SymbolicCounterexampleArtifact,
    /// Path the artifact was loaded from, used in diagnostics.
    pub path: PathBuf,
}

#[derive(Clone, Copy)]
pub(crate) struct TestFunctionMatcher<'a> {
    config: &'a Config,
    inline_config: &'a InlineConfig,
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

    /// Counts the fuzz test functions and runnable invariant campaign anchors of `abi` that
    /// match `filter` in the current network pass.
    pub(crate) fn count_fuzz_engine_targets(
        &self,
        filter: &dyn TestFilter,
        id: &ArtifactId,
        abi: &JsonAbi,
        multi_network: &MultiNetworkConfig,
    ) -> (usize, usize) {
        let contract_name = id.identifier();
        let matches_network_pass = |func: &Function| {
            function_matches_network_pass(
                &multi_network.all_override_networks,
                multi_network.pass_network.as_ref(),
                self.inline_config.network_for(&self.config.profile, &contract_name, &func.name),
            )
        };
        let fuzz = self
            .test_functions(contract_name.clone(), abi, |contract_id, func, kind| {
                matches!(kind, TestFunctionKind::FuzzTest { .. })
                    && filter.matches_test_function_kind_in_contract(contract_id, func, kind)
                    && matches_network_pass(func)
            })
            .count();
        let invariant = count_runnable_invariant_campaign_anchors(
            abi,
            filter,
            InvariantCampaignScope {
                config: self.config,
                inline_config: self.inline_config,
                contract_name: &contract_name,
                all_override_networks: &multi_network.all_override_networks,
                pass_network: multi_network.pass_network.as_ref(),
            },
        );
        (fuzz, invariant)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_symbolic_regression_detection_uses_marker() {
        let mut abi = JsonAbi::new();
        let ordinary = Function::parse("test_fails()").unwrap();
        abi.functions.entry(ordinary.name.clone()).or_default().push(ordinary);
        assert!(!is_generated_symbolic_regression_contract(&abi));

        let marker = Function::parse(&format!("{SYMBOLIC_REGRESSION_MARKER}()")).unwrap();
        abi.functions.entry(marker.name.clone()).or_default().push(marker);
        assert!(is_generated_symbolic_regression_contract(&abi));
    }
}
