//! Commonly used traits.

use alloy_json_abi::Function;
use alloy_primitives::Bytes;
use alloy_sol_types::SolError;
use auto_impl::auto_impl;

/// Extension trait for matching tests
#[auto_impl(&)]
pub trait TestFilter: Send + Sync {
    /// Returns whether the test should be included
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool;
    /// Returns whether the contract should be included
    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool;
    /// Returns a contract with the given path should be included
    fn matches_path(&self, path: impl AsRef<str>) -> bool;
}

/// Extension trait for `Function`
#[auto_impl(&)]
pub trait TestFunctionExt {
    /// Whether this function should be executed as invariant test
    fn is_invariant_test(&self) -> bool;
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
    fn is_invariant_test(&self) -> bool {
        self.name.is_invariant_test()
    }

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

impl TestFunctionExt for String {
    fn is_invariant_test(&self) -> bool {
        self.as_str().is_invariant_test()
    }

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

impl TestFunctionExt for str {
    fn is_invariant_test(&self) -> bool {
        self.starts_with("invariant") || self.starts_with("statefulFuzz")
    }

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
        self.eq_ignore_ascii_case("setup")
    }
}

/// An extension trait for `std::error::Error` for ABI encoding.
pub trait ErrorExt: std::error::Error {
    /// ABI-encodes the error using `Revert(string)`.
    fn abi_encode_revert(&self) -> Bytes;
}

impl<T: std::error::Error> ErrorExt for T {
    fn abi_encode_revert(&self) -> Bytes {
        alloy_sol_types::Revert::from(self.to_string()).abi_encode().into()
    }
}

/// Extension trait for matching functions
#[auto_impl(&)]
pub trait FunctionFilter {
    /// Returns whether the function should be included
    fn matches_function(&self, function_name: impl AsRef<str>) -> bool;
}