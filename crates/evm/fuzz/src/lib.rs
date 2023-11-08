//! # foundry-evm-fuzz
//!
//! EVM fuzzing implementation using [`proptest`].

#![warn(unreachable_pub, unused_crate_dependencies, rust_2018_idioms)]

#[macro_use]
extern crate tracing;

use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_primitives::{Address, Bytes, U256};
use ethers::types::Log;
use foundry_common::{calc, contracts::ContractsByAddress};
use foundry_evm_coverage::HitMaps;
use foundry_evm_traces::CallTraceArena;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

pub use proptest::test_runner::{Config as FuzzConfig, Reason};

mod error;
pub use error::FuzzError;

pub mod invariant;
pub mod strategies;

mod inspector;
pub use inspector::Fuzzer;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CounterExample {
    /// Call used as a counter example for fuzz tests.
    Single(BaseCounterExample),
    /// Sequence of calls used as a counter example for invariant tests.
    Sequence(Vec<BaseCounterExample>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseCounterExample {
    /// Address which makes the call
    pub sender: Option<Address>,
    /// Address to which to call to
    pub addr: Option<Address>,
    /// The data to provide
    pub calldata: Bytes,
    /// Function signature if it exists
    pub signature: Option<String>,
    /// Contract name if it exists
    pub contract_name: Option<String>,
    /// Traces
    pub traces: Option<CallTraceArena>,
    #[serde(skip)]
    pub args: Vec<DynSolValue>,
}

impl BaseCounterExample {
    pub fn create(
        sender: Address,
        addr: Address,
        bytes: &Bytes,
        contracts: &ContractsByAddress,
        traces: Option<CallTraceArena>,
    ) -> Self {
        if let Some((name, abi)) = &contracts.get(&addr) {
            if let Some(func) = abi.functions().find(|f| f.selector() == bytes[..4]) {
                // skip the function selector when decoding
                if let Ok(args) = func.abi_decode_input(&bytes[4..], false) {
                    return BaseCounterExample {
                        sender: Some(sender),
                        addr: Some(addr),
                        calldata: bytes.clone(),
                        signature: Some(func.signature()),
                        contract_name: Some(name.clone()),
                        traces,
                        args,
                    }
                }
            }
        }

        BaseCounterExample {
            sender: Some(sender),
            addr: Some(addr),
            calldata: bytes.clone(),
            signature: None,
            contract_name: None,
            traces,
            args: vec![],
        }
    }
}

impl fmt::Display for BaseCounterExample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args = foundry_common::fmt::format_tokens(&self.args).collect::<Vec<_>>().join(", ");

        if let Some(sender) = self.sender {
            write!(f, "sender={sender} addr=")?
        }

        if let Some(name) = &self.contract_name {
            write!(f, "[{name}]")?
        }

        if let Some(addr) = &self.addr {
            write!(f, "{addr} ")?
        }

        if let Some(sig) = &self.signature {
            write!(f, "calldata={sig}")?
        } else {
            write!(f, "calldata={}", self.calldata)?
        }

        write!(f, ", args=[{args}]")
    }
}

/// The outcome of a fuzz test
#[derive(Debug)]
pub struct FuzzTestResult {
    /// we keep this for the debugger
    pub first_case: FuzzCase,
    /// Gas usage (gas_used, call_stipend) per cases
    pub gas_by_case: Vec<(u64, u64)>,
    /// Whether the test case was successful. This means that the transaction executed
    /// properly, or that there was a revert and that the test was expected to fail
    /// (prefixed with `testFail`)
    pub success: bool,

    /// If there was a revert, this field will be populated. Note that the test can
    /// still be successful (i.e self.success == true) when it's expected to fail.
    pub reason: Option<String>,

    /// Minimal reproduction test case for failing fuzz tests
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<Log>,

    /// The decoded DSTest logging events and Hardhat's `console.log` from [logs](Self::logs).
    pub decoded_logs: Vec<String>,

    /// Labeled addresses
    pub labeled_addresses: BTreeMap<Address, String>,

    /// Exemplary traces for a fuzz run of the test function
    ///
    /// **Note** We only store a single trace of a successful fuzz call, otherwise we would get
    /// `num(fuzz_cases)` traces, one for each run, which is neither helpful nor performant.
    pub traces: Option<CallTraceArena>,

    /// Raw coverage info
    pub coverage: Option<HitMaps>,
}

impl FuzzTestResult {
    /// Returns the median gas of all test cases
    pub fn median_gas(&self, with_stipend: bool) -> u64 {
        let mut values =
            self.gas_values(with_stipend).into_iter().map(U256::from).collect::<Vec<_>>();
        values.sort_unstable();
        calc::median_sorted(&values).to::<u64>()
    }

    /// Returns the average gas use of all test cases
    pub fn mean_gas(&self, with_stipend: bool) -> u64 {
        let mut values =
            self.gas_values(with_stipend).into_iter().map(U256::from).collect::<Vec<_>>();
        values.sort_unstable();
        calc::mean(&values).to::<u64>()
    }

    fn gas_values(&self, with_stipend: bool) -> Vec<u64> {
        self.gas_by_case
            .iter()
            .map(|gas| if with_stipend { gas.0 } else { gas.0.saturating_sub(gas.1) })
            .collect()
    }
}

/// Data of a single fuzz test case
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FuzzCase {
    /// The calldata used for this fuzz test
    pub calldata: Bytes,
    /// Consumed gas
    pub gas: u64,
    /// The initial gas stipend for the transaction
    pub stipend: u64,
}

/// Container type for all successful test cases
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FuzzedCases {
    cases: Vec<FuzzCase>,
}

impl FuzzedCases {
    #[inline]
    pub fn new(mut cases: Vec<FuzzCase>) -> Self {
        cases.sort_by_key(|c| c.gas);
        Self { cases }
    }

    #[inline]
    pub fn cases(&self) -> &[FuzzCase] {
        &self.cases
    }

    #[inline]
    pub fn into_cases(self) -> Vec<FuzzCase> {
        self.cases
    }

    /// Get the last [FuzzCase]
    #[inline]
    pub fn last(&self) -> Option<&FuzzCase> {
        self.cases.last()
    }

    /// Returns the median gas of all test cases
    #[inline]
    pub fn median_gas(&self, with_stipend: bool) -> u64 {
        let mut values =
            self.gas_values(with_stipend).into_iter().map(U256::from).collect::<Vec<_>>();
        values.sort_unstable();
        calc::median_sorted(&values).to::<u64>()
    }

    /// Returns the average gas use of all test cases
    #[inline]
    pub fn mean_gas(&self, with_stipend: bool) -> u64 {
        let mut values =
            self.gas_values(with_stipend).into_iter().map(U256::from).collect::<Vec<_>>();
        values.sort_unstable();
        calc::mean(&values).to::<u64>()
    }

    #[inline]
    fn gas_values(&self, with_stipend: bool) -> Vec<u64> {
        self.cases
            .iter()
            .map(|c| if with_stipend { c.gas } else { c.gas.saturating_sub(c.stipend) })
            .collect()
    }

    /// Returns the case with the highest gas usage
    #[inline]
    pub fn highest(&self) -> Option<&FuzzCase> {
        self.cases.last()
    }

    /// Returns the case with the lowest gas usage
    #[inline]
    pub fn lowest(&self) -> Option<&FuzzCase> {
        self.cases.first()
    }

    /// Returns the highest amount of gas spent on a fuzz case
    #[inline]
    pub fn highest_gas(&self, with_stipend: bool) -> u64 {
        self.highest()
            .map(|c| if with_stipend { c.gas } else { c.gas - c.stipend })
            .unwrap_or_default()
    }

    /// Returns the lowest amount of gas spent on a fuzz case
    #[inline]
    pub fn lowest_gas(&self) -> u64 {
        self.lowest().map(|c| c.gas).unwrap_or_default()
    }
}
