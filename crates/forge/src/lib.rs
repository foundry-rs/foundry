#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate foundry_common;

#[macro_use]
extern crate tracing;

use alloy_primitives::U256;
use foundry_compilers::ProjectCompileOutput;
use foundry_config::{
    figment::{self, Figment},
    Config, FuzzConfig, InlineConfig, InvariantConfig, NatSpec,
};
use proptest::test_runner::{
    FailurePersistence, FileFailurePersistence, RngAlgorithm, TestRng, TestRunner,
};
use std::sync::Arc;

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

/// Test configuration.
#[derive(Clone, Debug, Default)]
pub struct TestOptions {
    /// The base configuration.
    pub config: Arc<Config>,
    /// Per-test configuration. Merged onto `base_config`.
    pub inline: InlineConfig,
}

impl TestOptions {
    /// Tries to create a new instance by detecting inline configurations from the project compile
    /// output.
    pub fn new(output: &ProjectCompileOutput, base_config: Arc<Config>) -> eyre::Result<Self> {
        let natspecs: Vec<NatSpec> = NatSpec::parse(output, &base_config.root);
        let profiles = &base_config.profiles;
        let mut inline = InlineConfig::new();
        for natspec in &natspecs {
            inline.insert(natspec)?;
            // Validate after parsing as TOML.
            natspec.validate_profiles(profiles)?;
        }
        Ok(Self { config: base_config, inline })
    }

    /// Creates a new instance without parsing inline configuration.
    pub fn new_unparsed(base_config: Arc<Config>) -> Self {
        Self { config: base_config, inline: InlineConfig::new() }
    }

    /// Returns the [`Figment`] for the configuration.
    pub fn figment(&self, contract_id: &str, test_fn: &str) -> Figment {
        self.inline.merge(contract_id, test_fn, &self.config)
    }

    /// Returns a "fuzz" test runner instance. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn fuzz_runner(
        &self,
        contract_id: &str,
        test_fn: &str,
    ) -> figment::Result<(FuzzConfig, TestRunner)> {
        let config: FuzzConfig = self.figment(contract_id, test_fn).extract()?;
        let failure_persist_path = config
            .failure_persist_dir
            .as_ref()
            .unwrap()
            .join(config.failure_persist_file.as_ref().unwrap())
            .into_os_string()
            .into_string()
            .unwrap();
        let runner = Self::fuzzer_with_cases(
            config.seed,
            config.runs,
            config.max_test_rejects,
            Some(Box::new(FileFailurePersistence::Direct(failure_persist_path.leak()))),
        );
        Ok((config, runner))
    }

    /// Returns an "invariant" test runner instance. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn invariant_runner(
        &self,
        contract_id: &str,
        test_fn: &str,
    ) -> figment::Result<(InvariantConfig, TestRunner)> {
        let figment = self.figment(contract_id, test_fn);
        let config: InvariantConfig = figment.extract()?;
        let seed: Option<U256> = figment.extract_inner("fuzz.seed").ok();
        let runner = Self::fuzzer_with_cases(seed, config.runs, config.max_assume_rejects, None);
        Ok((config, runner))
    }

    fn fuzzer_with_cases(
        seed: Option<U256>,
        cases: u32,
        max_global_rejects: u32,
        file_failure_persistence: Option<Box<dyn FailurePersistence>>,
    ) -> TestRunner {
        let config = proptest::test_runner::Config {
            failure_persistence: file_failure_persistence,
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
}
