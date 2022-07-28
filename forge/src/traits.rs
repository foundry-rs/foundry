use ethers::abi::Function;

/// Extension trait for matching tests
pub trait TestFilter: Send + Sync {
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool;
    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool;
    fn matches_path(&self, path: impl AsRef<str>) -> bool;
}

/// Extension trait for `Function`
pub(crate) trait TestFunctionExt {
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
        self.name.starts_with("test")
    }

    fn is_test_fail(&self) -> bool {
        self.name.starts_with("testFail")
    }

    fn is_setup(&self) -> bool {
        self.name.to_lowercase() == "setup"
    }
}
