use std::path::Path;

use ethers::solc::ProjectCompileOutput;
use foundry_config::{ConfParserError, FuzzConfig, InlineConfig, InvariantConfig};
use proptest::test_runner::{RngAlgorithm, TestRng, TestRunner};
use tracing::trace;

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
    pub fn invariant_fuzzer(&self) -> TestRunner {
        self.fuzzer_with_cases(self.invariant.runs)
    }

    pub fn fuzzer(&self) -> TestRunner {
        self.fuzzer_with_cases(self.fuzz.runs)
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

/// Builder utility to create a [`TestOptions`] instance.
#[derive(Default)]
pub struct TestOptionsBuilder {
    fuzz: Option<FuzzConfig>,
    invariant: Option<InvariantConfig>,
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
    pub fn build(self, root: impl AsRef<Path>) -> Result<TestOptions, ConfParserError> {
        let base_fuzz = self.fuzz.unwrap_or_default();
        let base_invariant = self.invariant.unwrap_or_default();

        match self.output {
            Some(compile_output) => Ok(TestOptions {
                fuzz: base_fuzz,
                invariant: base_invariant,
                inline_fuzz: InlineConfig::try_from((&compile_output, &base_fuzz, &root))?,
                inline_invariant: InlineConfig::try_from((
                    &compile_output,
                    &base_invariant,
                    &root,
                ))?,
            }),
            None => Ok(TestOptions {
                fuzz: base_fuzz,
                invariant: base_invariant,
                inline_fuzz: InlineConfig::default(),
                inline_invariant: InlineConfig::default(),
            }),
        }
    }
}
