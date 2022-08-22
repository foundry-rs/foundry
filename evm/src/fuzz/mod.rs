use crate::{
    decode,
    executor::{Executor, RawCallResult},
    trace::CallTraceArena,
};
use ethers::{
    abi::{Abi, Function, Token},
    types::{Address, Bytes, Log},
};
use foundry_common::{calc, contracts::ContractsByAddress};
pub use proptest::test_runner::{Config as FuzzConfig, Reason};
use proptest::test_runner::{TestCaseError, TestError, TestRunner};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, fmt};
use strategies::{
    build_initial_state, collect_state_from_call, fuzz_calldata, fuzz_calldata_from_state,
    EvmFuzzState,
};
pub mod invariant;
pub mod strategies;

/// Magic return code for the `assume` cheatcode
pub const ASSUME_MAGIC_RETURN_CODE: &[u8] = b"FOUNDRY::ASSUME";

/// Wrapper around an [`Executor`] which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
pub struct FuzzedExecutor<'a> {
    /// The VM
    executor: &'a Executor,
    /// The fuzzer
    runner: TestRunner,
    /// The account that calls tests
    sender: Address,
}

impl<'a> FuzzedExecutor<'a> {
    /// Instantiates a fuzzed executor given a testrunner
    pub fn new(executor: &'a Executor, runner: TestRunner, sender: Address) -> Self {
        Self { executor, runner, sender }
    }

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case
    pub fn fuzz(
        &self,
        func: &Function,
        address: Address,
        should_fail: bool,
        errors: Option<&Abi>,
    ) -> FuzzTestResult {
        // Stores the consumed gas and calldata of every successful fuzz call
        let cases: RefCell<Vec<FuzzCase>> = RefCell::default();

        // Stores the result and calldata of the last failed call, if any.
        let counterexample: RefCell<(Bytes, RawCallResult)> = RefCell::default();

        // stores the last successful call trace
        let traces: RefCell<Option<CallTraceArena>> = RefCell::default();

        // Stores fuzz state for use with [fuzz_calldata_from_state]
        let state: EvmFuzzState = if let Some(fork_db) = self.executor.backend().active_fork_db() {
            build_initial_state(fork_db)
        } else {
            build_initial_state(self.executor.backend().mem_db())
        };

        // TODO: We should have a `FuzzerOpts` struct where we can configure the fuzzer. When we
        // have that, we should add a way to configure strategy weights
        let strat = proptest::strategy::Union::new_weighted(vec![
            (60, fuzz_calldata(func.clone())),
            (40, fuzz_calldata_from_state(func.clone(), state.clone())),
        ]);
        tracing::debug!(func = ?func.name, should_fail, "fuzzing");
        let run_result = self.runner.clone().run(&strat, |calldata| {
            let call = self
                .executor
                .call_raw(self.sender, address, calldata.0.clone(), 0.into())
                .expect("Could not call contract with fuzzed input.");
            let state_changeset =
                call.state_changeset.as_ref().expect("We should have a state changeset.");

            // Build fuzzer state
            collect_state_from_call(&call.logs, state_changeset, state.clone());

            // When assume cheat code is triggered return a special string "FOUNDRY::ASSUME"
            if call.result.as_ref() == ASSUME_MAGIC_RETURN_CODE {
                return Err(TestCaseError::reject("ASSUME: Too many rejects"))
            }

            let success = self.executor.is_success(
                address,
                call.reverted,
                state_changeset.clone(),
                should_fail,
            );

            if success {
                cases.borrow_mut().push(FuzzCase {
                    calldata,
                    gas: call.gas_used,
                    stipend: call.stipend,
                });

                traces.replace(call.traces);

                Ok(())
            } else {
                let status = call.exit_reason;
                // We cannot use the calldata returned by the test runner in `TestError::Fail`,
                // since that input represents the last run case, which may not correspond with our
                // failure - when a fuzz case fails, proptest will try to run at least one more
                // case to find a minimal failure case.
                *counterexample.borrow_mut() = (calldata, call);
                Err(TestCaseError::fail(
                    match decode::decode_revert(
                        counterexample.borrow().1.result.as_ref(),
                        errors,
                        Some(status),
                    ) {
                        Ok(e) => e,
                        Err(_) => "".to_string(),
                    },
                ))
            }
        });

        tracing::trace!(target: "forge::test::fuzz::dictionary", "{:?}", state.read().iter().map(hex::encode).collect::<Vec<_>>());

        let (calldata, call) = counterexample.into_inner();
        let mut result = FuzzTestResult {
            cases: FuzzedCases::new(cases.into_inner()),
            success: run_result.is_ok(),
            reason: None,
            counterexample: None,
            logs: call.logs,
            labeled_addresses: call.labels,
            traces: traces.into_inner(),
        };

        match run_result {
            Err(TestError::Abort(reason)) => {
                result.reason = Some(reason.to_string());
            }
            Err(TestError::Fail(reason, _)) => {
                let reason = reason.to_string();
                result.reason = if reason.is_empty() { None } else { Some(reason) };

                let args = func
                    .decode_input(&calldata.as_ref()[4..])
                    .expect("could not decode fuzzer inputs");

                result.counterexample = Some(CounterExample::Single(BaseCounterExample {
                    sender: None,
                    addr: None,
                    signature: None,
                    contract_name: None,
                    traces: call.traces,
                    calldata,
                    args,
                }));
            }
            _ => (),
        }

        result
    }
}

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
    // Token does not implement Serde (lol), so we just serialize the calldata
    #[serde(skip)]
    pub args: Vec<Token>,
}

impl BaseCounterExample {
    pub fn create(
        sender: Address,
        addr: Address,
        bytes: &Bytes,
        contracts: &ContractsByAddress,
        traces: Option<CallTraceArena>,
    ) -> Self {
        let (name, abi) = &contracts.get(&addr).expect("Couldnt call unknown contract");

        let func = abi
            .functions()
            .find(|f| f.short_signature() == bytes.0.as_ref()[0..4])
            .expect("Couldnt find function");

        // skip the function selector when decoding
        let args = func.decode_input(&bytes.0.as_ref()[4..]).expect("Unable to decode input");

        BaseCounterExample {
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

impl fmt::Display for BaseCounterExample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args = foundry_utils::format_tokens(&self.args).collect::<Vec<_>>().join(", ");

        if let Some(sender) = self.sender {
            write!(f, "sender={:?} addr=", sender)?
        }

        if let Some(name) = &self.contract_name {
            write!(f, "[{}]", name)?
        }

        if let Some(addr) = &self.addr {
            write!(f, "{:?} ", addr)?
        }

        if let Some(sig) = &self.signature {
            write!(f, "calldata={}", &sig)?
        } else {
            write!(f, "calldata=0x{}", hex::encode(&self.calldata))?
        }

        write!(f, ", args=[{}]", args)
    }
}

/// The outcome of a fuzz test
#[derive(Debug)]
pub struct FuzzTestResult {
    /// Every successful fuzz test case
    pub cases: FuzzedCases,

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

    /// Labeled addresses
    pub labeled_addresses: BTreeMap<Address, String>,

    /// Exemplary traces for a fuzz run of the test function
    ///
    /// **Note** We only store a single trace of a successful fuzz call, otherwise we would get
    /// `num(fuzz_cases)` traces, one for each run, which is neither helpful nor performant.
    pub traces: Option<CallTraceArena>,
}

/// Container type for all successful test cases
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FuzzedCases {
    cases: Vec<FuzzCase>,
}

impl FuzzedCases {
    pub fn new(mut cases: Vec<FuzzCase>) -> Self {
        cases.sort_by_key(|c| c.gas);
        Self { cases }
    }

    pub fn cases(&self) -> &[FuzzCase] {
        &self.cases
    }

    pub fn into_cases(self) -> Vec<FuzzCase> {
        self.cases
    }

    /// Get the last [FuzzCase]
    pub fn last(&self) -> Option<&FuzzCase> {
        self.cases.last()
    }

    /// Returns the median gas of all test cases
    pub fn median_gas(&self, with_stipend: bool) -> u64 {
        let mut values = self.gas_values(with_stipend);
        values.sort_unstable();
        calc::median_sorted(&values)
    }

    /// Returns the average gas use of all test cases
    pub fn mean_gas(&self, with_stipend: bool) -> u64 {
        let mut values = self.gas_values(with_stipend);
        values.sort_unstable();
        calc::mean(&values).as_u64()
    }

    fn gas_values(&self, with_stipend: bool) -> Vec<u64> {
        self.cases.iter().map(|c| if with_stipend { c.gas } else { c.gas - c.stipend }).collect()
    }

    /// Returns the case with the highest gas usage
    pub fn highest(&self) -> Option<&FuzzCase> {
        self.cases.last()
    }

    /// Returns the case with the lowest gas usage
    pub fn lowest(&self) -> Option<&FuzzCase> {
        self.cases.first()
    }

    /// Returns the highest amount of gas spent on a fuzz case
    pub fn highest_gas(&self, with_stipend: bool) -> u64 {
        self.highest()
            .map(|c| if with_stipend { c.gas } else { c.gas - c.stipend })
            .unwrap_or_default()
    }

    /// Returns the lowest amount of gas spent on a fuzz case
    pub fn lowest_gas(&self) -> u64 {
        self.lowest().map(|c| c.gas).unwrap_or_default()
    }
}

/// Data of a single fuzz test case
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FuzzCase {
    /// The calldata used for this fuzz test
    pub calldata: Bytes,
    /// Consumed gas
    pub gas: u64,
    /// The initial gas stipend for the transaction
    pub stipend: u64,
}
