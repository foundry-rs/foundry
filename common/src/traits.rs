//! Commonly used traits

use ethers_core::abi::Function;

/// Extension trait for matching tests
pub trait TestFilter: Send + Sync {
    /// Returns whether the test should be included
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool;
    /// Returns whether the contract should be included
    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool;
    /// Returns a contract with the given path should be included
    fn matches_path(&self, path: impl AsRef<str>) -> bool;
}

/// Extension trait for `Function`
pub trait TestFunctionExt {
    /// Whether this function should be executed as fuzz test
    fn is_fuzz_test(&self) -> bool;
    /// Whether this function is a test
    fn is_test(&self) -> bool;
    /// Whether this function is a test that should fail
    fn is_test_fail(&self) -> bool;
    /// Whether this function is a `setUp` function
    fn is_setup(&self) -> bool;
}

impl TestFunctionExt for Function {
    fn is_fuzz_test(&self) -> bool {
        // test functions that have inputs are considered fuzz tests as those inputs will be fuzzed
        !self.inputs.is_empty()
    }

    fn is_test(&self) -> bool {
        self.name.is_test()
    }

    fn is_test_fail(&self) -> bool {
        self.name.is_test_fail()
    }

    fn is_setup(&self) -> bool {
        self.name.is_setup()
    }
}

impl<'a> TestFunctionExt for &'a str {
    fn is_fuzz_test(&self) -> bool {
        unimplemented!("no naming convention for fuzz tests.")
    }

    fn is_test(&self) -> bool {
        self.starts_with("test")
    }

    fn is_test_fail(&self) -> bool {
        self.starts_with("testFail")
    }

    fn is_setup(&self) -> bool {
        self.to_lowercase() == "setup"
    }
}

impl TestFunctionExt for String {
    fn is_fuzz_test(&self) -> bool {
        self.as_str().is_fuzz_test()
    }

    fn is_test(&self) -> bool {
        self.as_str().is_test()
    }

    fn is_test_fail(&self) -> bool {
        self.as_str().is_test_fail()
    }

    fn is_setup(&self) -> bool {
        self.as_str().is_setup()
    }
}
