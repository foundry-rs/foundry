// forge/tests/it/rust.rs

// Import necessary components from the test helpers and the forge crate itself.
use crate::{
    config::*, // Provides TestConfig for running tests and assert_multiple for checking results.
    test_helpers::{ForgeTestData, ForgeTestProfile}, // Utilities for setting up test projects.
};
use forge::{result::TestStatus, MultiContractRunner}; // The core test runner.
use foundry_test_utils::Filter; // Used to select which tests to run.
use std::{collections::BTreeMap, fs, path::PathBuf, sync::LazyLock};


const RUST_CONTRACT_CODE: &str = r#"
#![cfg_attr(not(feature = "std"), no_std, no_main)]

extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    SharedAPI, U256,
};

#[derive(Contract, Default)]
struct PowerCalculator<SDK> {
    sdk: SDK,
}

pub trait PowerAPI {
    /// Calculate base^exponent
    fn power(&self, base: U256, exponent: U256) -> U256;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> PowerAPI for PowerCalculator<SDK> {
    fn power(&self, base: U256, exponent: U256) -> U256 {
        if exponent == U256::from(0) {
            return U256::from(1);
        }
        let mut result = U256::from(1);
        let mut exp = exponent;
        let mut base_pow = base;
        while exp > U256::from(0) {
            if exp & U256::from(1) == U256::from(1) {
                result = result * base_pow;
            }
            base_pow = base_pow * base_pow;
            exp = exp >> 1;
        }
        result
    }
}

impl<SDK: SharedAPI> PowerCalculator<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(PowerCalculator);
"#;

// --- Contents of Cargo.toml for our Rust contract ---
const RUST_CONTRACT_CARGO_TOML: &str = r#"
[package]
name = "power-calculator"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
# IMPORTANT: The path to the SDK must be correct for `fluentbase-build` to work.
# This relative path assumes a specific directory structure.
# You might need to adjust it based on your monorepo layout.
fluentbase-sdk = { git = "https://github.com/fluentlabs-xyz/fluentbase", tag = "v0.3.6-dev", default-features=false }

[features]
default = ["std"]
std = ["fluentbase-sdk/std"]
wasm = []

[profile.release]
opt-level = "z"
lto = true
panic = "abort"
codegen-units = 1

# Exclude from foundry workspace
[workspace]
"#;

// --- Source code for the Solidity test that will call the Rust contract ---
const SOLIDITY_TEST_CODE: &str = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
// This import is crucial. It points to the Solidity interface file that
// our `fluentbase-build` integration will generate during compilation.
import "../src/power-calculator/interface.sol";

contract PowerCalculatorTest is Test {
    function test_CallRustPowerCalculator() public {
        // Deploy the Rust contract as if it were a normal Solidity contract.
        // The `new` keyword triggers the deployment process. `forge` will see
        // the WASM bytecode in the artifact and hand it to your patched revm.
        PowerCalculator calculator = new PowerCalculator();
        
        // Call the `power` function on the deployed Rust contract.
        uint256 result = calculator.power(2, 10);
        
        // Assert that the result is correct (2^10 = 1024).
        assertEq(result, 1024);

        // Test another case
        uint256 result2 = calculator.power(3, 3);
        assertEq(result2, 27);
    }
}
"#;

/// A helper struct to manage the test project's data and setup.
struct RustProjectTestData {
    /// Contains the base Foundry project configuration and paths.
    base: ForgeTestData,
}

impl RustProjectTestData {
    /// Creates a new test project, complete with a Rust contract and a Solidity test.
    fn new() -> Self {
        // Use ForgeTestData::new to create a standard Foundry project layout.
        let base = ForgeTestData::new(ForgeTestProfile::Default);
        let root = base.project.root();

        // --- Create the directory structure and files for the Rust contract ---
        let rust_contract_dir = root.join("src/power-calculator");
        fs::create_dir_all(rust_contract_dir.join("src")).unwrap();
        
        fs::write(rust_contract_dir.join("Cargo.toml"), RUST_CONTRACT_CARGO_TOML).unwrap();
        fs::write(rust_contract_dir.join("src/lib.rs"), RUST_CONTRACT_CODE).unwrap();

        // --- Create the Solidity test file ---
        let test_contract_path = root.join("test/PowerCalculatorTest.t.sol");
        fs::write(&test_contract_path, SOLIDITY_TEST_CODE).unwrap();

        Self { base }
    }

    /// Gets a `MultiContractRunner` for our project.
    /// This is the main entry point for running tests programmatically.
    /// Calling this method will trigger the compilation process, including our
    /// custom Rust build logic inside `compile.rs`.
    fn runner(&self) -> MultiContractRunner {
        self.base.runner()
    }
}

// Use `LazyLock` to ensure the test project is set up only once.
// This is a performance optimization that is standard in Foundry's own tests.
static TEST_DATA_RUST: LazyLock<RustProjectTestData> = LazyLock::new(RustProjectTestData::new);

// --- The main integration test function ---
#[tokio::test(flavor = "multi_thread")]
async fn test_can_compile_and_test_rust_contract_from_solidity() {
    // 1. Get the `MultiContractRunner`. This implicitly runs `forge build`.
    // If the compilation (including the Rust part) fails, this call will panic.
    let mut runner = TEST_DATA_RUST.runner();

    // 2. Define a filter to run only our specific Solidity test.
    let filter = Filter::new(".*", "PowerCalculatorTest", ".*PowerCalculatorTest.t.sol");

    // 3. Execute the test(s) and collect the results.
    let results = runner.test_collect(&filter).expect("Test collection failed");
    
    // 4. Assert the results to ensure everything worked as expected.
    assert_multiple(
        &results,
        BTreeMap::from([(
            // The key is the test suite identifier: "profile/path/to/file:ContractName"
            "default/test/PowerCalculatorTest.t.sol:PowerCalculatorTest",
            // The value is a vector of test function results to check.
            // Format: (test_signature, should_pass, expected_reason, expected_logs, expected_warnings)
            vec![(
                "test_CallRustPowerCalculator()",
                true,       // We expect this test to pass.
                None,       // No revert reason is expected.
                None,       // We don't check for specific logs.
                None,       // We don't check for specific warnings.
            )],
        )]),
    );
}