//! test outcomes

use crate::Address;
use ethers::prelude::Log;
use foundry_evm::{
    coverage::HitMaps,
    fuzz::{CounterExample, FuzzCase},
    trace::Traces,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, time::Duration};

/// Results and duration for a set of tests included in the same test contract
#[derive(Debug, Clone, Serialize)]
pub struct SuiteResult {
    /// Total duration of the test run for this block of tests
    pub duration: Duration,
    /// Individual test results. `test method name -> TestResult`
    pub test_results: BTreeMap<String, TestResult>,
    /// Warnings
    pub warnings: Vec<String>,
}

impl SuiteResult {
    pub fn new(
        duration: Duration,
        test_results: BTreeMap<String, TestResult>,
        warnings: Vec<String>,
    ) -> Self {
        Self { duration, test_results, warnings }
    }

    /// Iterator over all succeeding tests and their names
    pub fn successes(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.success)
    }

    /// Iterator over all failing tests and their names
    pub fn failures(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| !t.success)
    }

    /// Iterator over all tests and their names
    pub fn tests(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.test_results.iter()
    }

    /// Whether this test suite is empty.
    pub fn is_empty(&self) -> bool {
        self.test_results.is_empty()
    }

    /// The number of tests in this test suite.
    pub fn len(&self) -> usize {
        self.test_results.len()
    }
}

/// The result of an executed solidity test
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestResult {
    /// Whether the test case was successful. This means that the transaction executed
    /// properly, or that there was a revert and that the test was expected to fail
    /// (prefixed with `testFail`)
    pub success: bool,

    /// If there was a revert, this field will be populated. Note that the test can
    /// still be successful (i.e self.success == true) when it's expected to fail.
    pub reason: Option<String>,

    /// Minimal reproduction test case for failing test
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<Log>,

    /// The decoded DSTest logging events and Hardhat's `console.log` from [logs](Self::logs).
    pub decoded_logs: Vec<String>,

    /// What kind of test this was
    pub kind: TestKind,

    /// Traces
    pub traces: Traces,

    /// Raw coverage info
    #[serde(skip)]
    pub coverage: Option<HitMaps>,

    /// Labeled addresses
    pub labeled_addresses: BTreeMap<Address, String>,
}

impl TestResult {
    /// Returns `true` if this is the result of a fuzz test
    pub fn is_fuzz(&self) -> bool {
        matches!(self.kind, TestKind::Fuzz { .. })
    }
}

/// Data report by a test.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TestKindReport {
    Standard { gas: u64 },
    Fuzz { runs: usize, mean_gas: u64, median_gas: u64 },
    Invariant { runs: usize, calls: usize, reverts: usize },
}

impl fmt::Display for TestKindReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestKindReport::Standard { gas } => {
                write!(f, "(gas: {gas})")
            }
            TestKindReport::Fuzz { runs, mean_gas, median_gas } => {
                write!(f, "(runs: {runs}, μ: {mean_gas}, ~: {median_gas})")
            }
            TestKindReport::Invariant { runs, calls, reverts } => {
                write!(f, "(runs: {runs}, calls: {calls}, reverts: {reverts})")
            }
        }
    }
}

impl TestKindReport {
    /// Returns the main gas value to compare against
    pub fn gas(&self) -> u64 {
        match self {
            TestKindReport::Standard { gas } => *gas,
            // We use the median for comparisons
            TestKindReport::Fuzz { median_gas, .. } => *median_gas,
            // We return 0 since it's not applicable
            TestKindReport::Invariant { .. } => 0,
        }
    }
}

/// Various types of tests
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TestKind {
    /// A standard test that consists of calling the defined solidity function
    ///
    /// Holds the consumed gas
    Standard(u64),
    /// A solidity fuzz test, that stores all test cases
    Fuzz {
        /// we keep this for the debugger
        first_case: FuzzCase,
        runs: usize,
        mean_gas: u64,
        median_gas: u64,
    },
    /// A solidity invariant test, that stores all test cases
    Invariant { runs: usize, calls: usize, reverts: usize },
}

impl TestKind {
    /// The gas consumed by this test
    pub fn report(&self) -> TestKindReport {
        match self {
            TestKind::Standard(gas) => TestKindReport::Standard { gas: *gas },
            TestKind::Fuzz { runs, mean_gas, median_gas, .. } => {
                TestKindReport::Fuzz { runs: *runs, mean_gas: *mean_gas, median_gas: *median_gas }
            }
            TestKind::Invariant { runs, calls, reverts } => {
                TestKindReport::Invariant { runs: *runs, calls: *calls, reverts: *reverts }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TestSetup {
    /// The address at which the test contract was deployed
    pub address: Address,
    /// The logs emitted during setup
    pub logs: Vec<Log>,
    /// Call traces of the setup
    pub traces: Traces,
    /// Addresses labeled during setup
    pub labeled_addresses: BTreeMap<Address, String>,
    /// Whether the setup failed
    pub setup_failed: bool,
    /// The reason the setup failed
    pub reason: Option<String>,
}
