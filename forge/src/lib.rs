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
#[derive(Debug, Clone, Copy, Default)]
pub struct TestOptions {
    /// The fuzz test configuration
    pub fuzz: foundry_config::FuzzConfig,
    /// The invariant test configuration
    pub invariant: foundry_config::InvariantConfig,
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
            proptest::test_runner::TestRunner::new_with_rng(cfg, rng)
        } else {
            trace!(target: "forge::test", "building stochastic fuzzer");
            proptest::test_runner::TestRunner::new(cfg)
        }
    }
}
