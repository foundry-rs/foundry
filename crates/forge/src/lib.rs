#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use foundry_compilers::ProjectCompileOutput;
use foundry_config::{
    validate_profiles, Config, FuzzConfig, InlineConfig, InlineConfigError, InlineConfigParser,
    InvariantConfig, NatSpec,
};
use proptest::test_runner::{
    FailurePersistence, FileFailurePersistence, RngAlgorithm, TestRng, TestRunner,
};
use std::path::Path;

pub mod coverage;

pub mod gas_report;

pub mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

mod runner;
pub use runner::ContractRunner;

mod progress;
pub mod result;

// TODO: remove
pub use foundry_common::traits::TestFilter;
pub use foundry_evm::*;

/// Metadata on how to run fuzz/invariant tests
#[derive(Clone, Debug, Default)]
pub struct TestOptions {
    /// The base "fuzz" test configuration. To be used as a fallback in case
    /// no more specific configs are found for a given run.
    pub fuzz: FuzzConfig,
    /// The base "invariant" test configuration. To be used as a fallback in case
    /// no more specific configs are found for a given run.
    pub invariant: InvariantConfig,
    /// Contains per-test specific "fuzz" configurations.
    pub inline_fuzz: InlineConfig<FuzzConfig>,
    /// Contains per-test specific "invariant" configurations.
    pub inline_invariant: InlineConfig<InvariantConfig>,
}

impl TestOptions {
    /// Tries to create a new instance by detecting inline configurations from the project compile
    /// output.
    pub fn new(
        output: &ProjectCompileOutput,
        root: &Path,
        profiles: Vec<String>,
        base_fuzz: FuzzConfig,
        base_invariant: InvariantConfig,
    ) -> Result<Self, InlineConfigError> {
        let natspecs: Vec<NatSpec> = NatSpec::parse(output, root);
        let mut inline_invariant = InlineConfig::<InvariantConfig>::default();
        let mut inline_fuzz = InlineConfig::<FuzzConfig>::default();

        for natspec in natspecs {
            // Perform general validation
            validate_profiles(&natspec, &profiles)?;
            FuzzConfig::validate_configs(&natspec)?;
            InvariantConfig::validate_configs(&natspec)?;

            // Apply in-line configurations for the current profile
            let configs: Vec<String> = natspec.current_profile_configs().collect();
            let c: &str = &natspec.contract;
            let f: &str = &natspec.function;
            let line: String = natspec.debug_context();

            match base_fuzz.try_merge(&configs) {
                Ok(Some(conf)) => inline_fuzz.insert(c, f, conf),
                Ok(None) => { /* No inline config found, do nothing */ }
                Err(e) => Err(InlineConfigError { line: line.clone(), source: e })?,
            }

            match base_invariant.try_merge(&configs) {
                Ok(Some(conf)) => inline_invariant.insert(c, f, conf),
                Ok(None) => { /* No inline config found, do nothing */ }
                Err(e) => Err(InlineConfigError { line: line.clone(), source: e })?,
            }
        }

        Ok(Self { fuzz: base_fuzz, invariant: base_invariant, inline_fuzz, inline_invariant })
    }

    /// Returns a "fuzz" test runner instance. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn fuzz_runner(&self, contract_id: &str, test_fn: &str) -> TestRunner {
        let fuzz_config = self.fuzz_config(contract_id, test_fn).clone();
        let failure_persist_path = fuzz_config
            .failure_persist_dir
            .unwrap()
            .join(fuzz_config.failure_persist_file.unwrap())
            .into_os_string()
            .into_string()
            .unwrap();
        self.fuzzer_with_cases(
            fuzz_config.runs,
            Some(Box::new(FileFailurePersistence::Direct(failure_persist_path.leak()))),
        )
    }

    /// Returns an "invariant" test runner instance. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn invariant_runner(&self, contract_id: &str, test_fn: &str) -> TestRunner {
        let invariant = self.invariant_config(contract_id, test_fn);
        self.fuzzer_with_cases(invariant.runs, None)
    }

    /// Returns a "fuzz" configuration setup. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn fuzz_config(&self, contract_id: &str, test_fn: &str) -> &FuzzConfig {
        self.inline_fuzz.get(contract_id, test_fn).unwrap_or(&self.fuzz)
    }

    /// Returns an "invariant" configuration setup. Parameters are used to select tight scoped
    /// invariant configs that apply for a contract-function pair. A fallback configuration is
    /// applied if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn invariant_config(&self, contract_id: &str, test_fn: &str) -> &InvariantConfig {
        self.inline_invariant.get(contract_id, test_fn).unwrap_or(&self.invariant)
    }

    pub fn fuzzer_with_cases(
        &self,
        cases: u32,
        file_failure_persistence: Option<Box<dyn FailurePersistence>>,
    ) -> TestRunner {
        let config = proptest::test_runner::Config {
            failure_persistence: file_failure_persistence,
            cases,
            max_global_rejects: self.fuzz.max_test_rejects,
            // Disable proptest shrink: for fuzz tests we provide single counterexample,
            // for invariant tests we shrink outside proptest.
            max_shrink_iters: 0,
            ..Default::default()
        };

        if let Some(seed) = &self.fuzz.seed {
            trace!(target: "forge::test", %seed, "building deterministic fuzzer");
            let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &seed.to_be_bytes::<32>());
            TestRunner::new_with_rng(config, rng)
        } else {
            trace!(target: "forge::test", "building stochastic fuzzer");
            TestRunner::new(config)
        }
    }
}

/// Builder utility to create a [`TestOptions`] instance.
#[derive(Default)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct TestOptionsBuilder {
    fuzz: Option<FuzzConfig>,
    invariant: Option<InvariantConfig>,
    profiles: Option<Vec<String>>,
}

impl TestOptionsBuilder {
    /// Sets a [`FuzzConfig`] to be used as base "fuzz" configuration.
    pub fn fuzz(mut self, conf: FuzzConfig) -> Self {
        self.fuzz = Some(conf);
        self
    }

    /// Sets a [`InvariantConfig`] to be used as base "invariant" configuration.
    pub fn invariant(mut self, conf: InvariantConfig) -> Self {
        self.invariant = Some(conf);
        self
    }

    /// Sets available configuration profiles. Profiles are useful to validate existing in-line
    /// configurations. This argument is necessary in case a `compile_output`is provided.
    pub fn profiles(mut self, p: Vec<String>) -> Self {
        self.profiles = Some(p);
        self
    }

    /// Creates an instance of [`TestOptions`]. This takes care of creating "fuzz" and
    /// "invariant" fallbacks, and extracting all inline test configs, if available.
    ///
    /// `root` is a reference to the user's project root dir. This is essential
    /// to determine the base path of generated contract identifiers. This is to provide correct
    /// matchers for inline test configs.
    pub fn build(
        self,
        output: &ProjectCompileOutput,
        root: &Path,
    ) -> Result<TestOptions, InlineConfigError> {
        let profiles: Vec<String> =
            self.profiles.unwrap_or_else(|| vec![Config::selected_profile().into()]);
        let base_fuzz = self.fuzz.unwrap_or_default();
        let base_invariant = self.invariant.unwrap_or_default();
        TestOptions::new(output, root, profiles, base_fuzz, base_invariant)
    }
}
