#![warn(unused_crate_dependencies)]

use ethers::solc::ProjectCompileOutput;
use foundry_config::{
    validate_profiles, Config, FuzzConfig, InlineConfig, InlineConfigError, InlineConfigParser,
    InvariantConfig, NatSpec,
};
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};
use std::path::Path;

#[macro_use]
extern crate tracing;

/// Gas reports
pub mod gas_report;

/// Coverage reports
pub mod coverage;

/// The Forge test runner
mod runner;
pub use runner::ContractRunner;

/// Forge test runners for multiple contracts
mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

/// reexport
pub use foundry_common::traits::TestFilter;

pub mod result;

/// The Forge EVM backend
pub use foundry_evm::*;

/// Metadata on how to run fuzz/invariant tests
#[derive(Debug, Clone, Default)]
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
    /// Returns a "fuzz" test runner instance. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn fuzz_runner<S>(&self, contract_id: S, test_fn: S) -> TestRunner
    where
        S: Into<String>,
    {
        let fuzz = self.fuzz_config(contract_id, test_fn);
        self.fuzzer_with_cases(fuzz.runs)
    }

    /// Returns an "invariant" test runner instance. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn invariant_runner<S>(&self, contract_id: S, test_fn: S) -> TestRunner
    where
        S: Into<String>,
    {
        let invariant = self.invariant_config(contract_id, test_fn);
        self.fuzzer_with_cases(invariant.runs)
    }

    /// Returns a "fuzz" configuration setup. Parameters are used to select tight scoped fuzz
    /// configs that apply for a contract-function pair. A fallback configuration is applied
    /// if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn fuzz_config<S>(&self, contract_id: S, test_fn: S) -> &FuzzConfig
    where
        S: Into<String>,
    {
        self.inline_fuzz.get(contract_id, test_fn).unwrap_or(&self.fuzz)
    }

    /// Returns an "invariant" configuration setup. Parameters are used to select tight scoped
    /// invariant configs that apply for a contract-function pair. A fallback configuration is
    /// applied if no specific setup is found for a given input.
    ///
    /// - `contract_id` is the id of the test contract, expressed as a relative path from the
    ///   project root.
    /// - `test_fn` is the name of the test function declared inside the test contract.
    pub fn invariant_config<S>(&self, contract_id: S, test_fn: S) -> &InvariantConfig
    where
        S: Into<String>,
    {
        self.inline_invariant.get(contract_id, test_fn).unwrap_or(&self.invariant)
    }

    pub fn fuzzer_with_cases(&self, cases: u32) -> TestRunner {
        // TODO: Add Options to modify the persistence
        let cfg = proptest::test_runner::Config {
            failure_persistence: None,
            cases,
            max_global_rejects: self.fuzz.max_test_rejects,
            ..Default::default()
        };

        if let Some(ref fuzz_seed) = self.fuzz.seed {
            trace!(target: "forge::test", "building deterministic fuzzer with seed {}", fuzz_seed);
            let mut bytes: [u8; 32] = [0; 32];
            fuzz_seed.to_big_endian(&mut bytes);
            let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &bytes);
            TestRunner::new_with_rng(cfg, rng)
        } else {
            trace!(target: "forge::test", "building stochastic fuzzer");
            TestRunner::new(cfg)
        }
    }
}

impl<'a, P> TryFrom<(&'a ProjectCompileOutput, &'a P, Vec<String>, FuzzConfig, InvariantConfig)>
    for TestOptions
where
    P: AsRef<Path>,
{
    type Error = InlineConfigError;

    /// Tries to create an instance of `Self`, detecting inline configurations from the project
    /// compile output.
    ///
    /// Param is a tuple, whose elements are:
    /// 1. Solidity compiler output, essential to extract natspec test configs.
    /// 2. Root path to express contract base dirs. This is essential to match inline configs at
    /// runtime. 3. List of available configuration profiles
    /// 4. Reference to a fuzz base configuration.
    /// 5. Reference to an invariant base configuration.
    fn try_from(
        value: (&'a ProjectCompileOutput, &'a P, Vec<String>, FuzzConfig, InvariantConfig),
    ) -> Result<Self, Self::Error> {
        let output = value.0;
        let root = value.1;
        let profiles = &value.2;
        let base_fuzz: FuzzConfig = value.3;
        let base_invariant: InvariantConfig = value.4;

        let natspecs: Vec<NatSpec> = NatSpec::parse(output, root);
        let mut inline_invariant = InlineConfig::<InvariantConfig>::default();
        let mut inline_fuzz = InlineConfig::<FuzzConfig>::default();

        for natspec in natspecs {
            // Perform general validation
            validate_profiles(&natspec, profiles)?;
            FuzzConfig::validate_configs(&natspec)?;
            InvariantConfig::validate_configs(&natspec)?;

            // Apply in-line configurations for the current profile
            let configs: Vec<String> = natspec.current_profile_configs();
            let c: &str = &natspec.contract;
            let f: &str = &natspec.function;
            let line: String = natspec.debug_context();

            match base_fuzz.try_merge(&configs) {
                Ok(Some(conf)) => inline_fuzz.insert(c, f, conf),
                Err(e) => Err(InlineConfigError { line: line.clone(), source: e })?,
                _ => { /* No inline config found, do nothing */ }
            }

            match base_invariant.try_merge(&configs) {
                Ok(Some(conf)) => inline_invariant.insert(c, f, conf),
                Err(e) => Err(InlineConfigError { line: line.clone(), source: e })?,
                _ => { /* No inline config found, do nothing */ }
            }
        }

        Ok(Self { fuzz: base_fuzz, invariant: base_invariant, inline_fuzz, inline_invariant })
    }
}

/// Builder utility to create a [`TestOptions`] instance.
#[derive(Default)]
pub struct TestOptionsBuilder {
    fuzz: Option<FuzzConfig>,
    invariant: Option<InvariantConfig>,
    profiles: Option<Vec<String>>,
    output: Option<ProjectCompileOutput>,
}

impl TestOptionsBuilder {
    /// Sets a [`FuzzConfig`] to be used as base "fuzz" configuration.
    #[must_use = "A base 'fuzz' config must be provided"]
    pub fn fuzz(mut self, conf: FuzzConfig) -> Self {
        self.fuzz = Some(conf);
        self
    }

    /// Sets a [`InvariantConfig`] to be used as base "invariant" configuration.
    #[must_use = "A base 'invariant' config must be provided"]
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

    /// Sets a project compiler output instance. This is used to extract
    /// inline test configurations that override `self.fuzz` and `self.invariant`
    /// specs when necessary.
    pub fn compile_output(mut self, output: &ProjectCompileOutput) -> Self {
        self.output = Some(output.clone());
        self
    }

    /// Creates an instance of [`TestOptions`]. This takes care of creating "fuzz" and
    /// "invariant" fallbacks, and extracting all inline test configs, if available.
    ///
    /// `root` is a reference to the user's project root dir. This is essential
    /// to determine the base path of generated contract identifiers. This is to provide correct
    /// matchers for inline test configs.
    pub fn build(self, root: impl AsRef<Path>) -> Result<TestOptions, InlineConfigError> {
        let default_profiles = vec![Config::selected_profile().into()];
        let profiles: Vec<String> = self.profiles.unwrap_or(default_profiles);
        let base_fuzz = self.fuzz.unwrap_or_default();
        let base_invariant = self.invariant.unwrap_or_default();

        match self.output {
            Some(compile_output) => Ok(TestOptions::try_from((
                &compile_output,
                &root,
                profiles,
                base_fuzz,
                base_invariant,
            ))?),
            None => Ok(TestOptions {
                fuzz: base_fuzz,
                invariant: base_invariant,
                inline_fuzz: InlineConfig::default(),
                inline_invariant: InlineConfig::default(),
            }),
        }
    }
}
