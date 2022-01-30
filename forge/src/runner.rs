use crate::TestFilter;
use evm_adapters::{
    evm_opts::EvmOpts,
    sputnik::{helpers::TestSputnikVM, Executor, PRECOMPILES_MAP},
};
use rayon::iter::ParallelIterator;
use sputnik::{backend::Backend, Config};

use ethers::{
    abi::{Abi, Event, Function, Token},
    types::{Address, Bytes, H256},
};
use evm_adapters::{
    call_tracing::CallTraceArena,
    fuzz::{FuzzTestResult, FuzzedCases, FuzzedExecutor},
    sputnik::cheatcodes::debugger::DebugArena,
    Evm, EvmError,
};
use eyre::Result;
use std::{collections::BTreeMap, fmt, time::Instant};

use proptest::test_runner::{TestError, TestRunner};
use rayon::iter::IntoParallelRefIterator;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CounterExample {
    pub calldata: Bytes,
    // Token does not implement Serde (lol), so we just serialize the calldata
    #[serde(skip)]
    pub args: Vec<Token>,
}

impl fmt::Display for CounterExample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args = foundry_utils::format_tokens(&self.args).collect::<Vec<_>>().join(", ");
        write!(f, "calldata=0x{}, args=[{}]", hex::encode(&self.calldata), args)
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

    /// The gas used during execution.
    ///
    /// If this is the result of a fuzz test (`TestKind::Fuzz`), then this is the median of all
    /// successful cases
    pub gas_used: u64,

    /// Minimal reproduction test case for failing fuzz tests
    pub counterexample: Option<CounterExample>,

    /// Any captured & parsed as strings logs along the test's execution which should
    /// be printed to the user.
    pub logs: Vec<String>,

    /// What kind of test this was
    pub kind: TestKind,

    /// Traces
    pub traces: Option<Vec<CallTraceArena>>,

    /// Identified contracts
    pub identified_contracts: Option<BTreeMap<Address, (String, Abi)>>,

    /// Debug Steps
    #[serde(skip)]
    pub debug_calls: Option<Vec<DebugArena>>,

    /// Labeled addresses
    pub labeled_addresses: BTreeMap<Address, String>,
}

impl TestResult {
    /// Returns `true` if this is the result of a fuzz test
    pub fn is_fuzz(&self) -> bool {
        matches!(self.kind, TestKind::Fuzz(_))
    }
}

/// Used gas by a test
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TestKindGas {
    Standard(u64),
    Fuzz { runs: usize, mean: u64, median: u64 },
}

impl fmt::Display for TestKindGas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestKindGas::Standard(gas) => {
                write!(f, "(gas: {})", gas)
            }
            TestKindGas::Fuzz { runs, mean, median } => {
                write!(f, "(runs: {}, μ: {}, ~: {})", runs, mean, median)
            }
        }
    }
}

impl TestKindGas {
    /// Returns the main gas value to compare against
    pub fn gas(&self) -> u64 {
        match self {
            TestKindGas::Standard(gas) => *gas,
            // we use the median for comparisons
            TestKindGas::Fuzz { median, .. } => *median,
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
    Fuzz(FuzzedCases),
}

impl TestKind {
    /// The gas consumed by this test
    pub fn gas_used(&self) -> TestKindGas {
        match self {
            TestKind::Standard(gas) => TestKindGas::Standard(*gas),
            TestKind::Fuzz(fuzzed) => TestKindGas::Fuzz {
                runs: fuzzed.cases().len(),
                median: fuzzed.median_gas(),
                mean: fuzzed.mean_gas(),
            },
        }
    }
}

/// Type complexity wrapper around execution info
type MaybeExecutionInfo<'a> =
    Option<(&'a BTreeMap<[u8; 4], Function>, &'a BTreeMap<H256, Event>, &'a Abi)>;

pub struct ContractRunner<'a, B> {
    // EVM Config Options
    /// The options used to instantiate a new EVM.
    pub evm_opts: &'a EvmOpts,
    /// The backend used by the VM.
    pub backend: &'a B,
    /// The VM Configuration to use for the runner (London, Berlin , ...)
    pub evm_cfg: &'a Config,

    // Contract deployment options
    /// The deployed contract's ABI
    pub contract: &'a Abi,
    /// The deployed contract's address
    // This is cheap to clone due to [`bytes::Bytes`], so OK to own
    pub code: ethers::prelude::Bytes,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Address,

    /// Contract execution info, (functions, events, errors)
    pub execution_info: MaybeExecutionInfo<'a>,
}

impl<'a, B: Backend> ContractRunner<'a, B> {
    pub fn new(
        evm_opts: &'a EvmOpts,
        evm_cfg: &'a Config,
        backend: &'a B,
        contract: &'a Abi,
        code: ethers::prelude::Bytes,
        sender: Option<Address>,
        execution_info: MaybeExecutionInfo<'a>,
    ) -> Self {
        Self {
            evm_opts,
            evm_cfg,
            backend,
            contract,
            code,
            sender: sender.unwrap_or_default(),
            execution_info,
        }
    }
}

// Require that the backend is Cloneable. This allows us to use the `SharedBackend` from
// evm-adapters which is clone-able.
impl<'a, B: Backend + Clone + Send + Sync> ContractRunner<'a, B> {
    /// Creates a new EVM and deploys the test contract inside the runner
    /// from the sending account.
    pub fn new_sputnik_evm(&'a self) -> eyre::Result<(Address, TestSputnikVM<'a, B>, Vec<String>)> {
        // create the EVM, clone the backend.
        let mut executor = Executor::new_with_cheatcodes(
            self.backend.clone(),
            self.evm_opts.env.gas_limit,
            self.evm_cfg,
            &*PRECOMPILES_MAP,
            self.evm_opts.ffi,
            self.evm_opts.verbosity > 2,
            self.evm_opts.debug,
        );

        // deploy an instance of the contract inside the runner in the EVM
        let (addr, _, _, logs) =
            executor.deploy(self.sender, self.code.clone(), 0u32.into()).expect("couldn't deploy");
        executor.set_balance(addr, self.evm_opts.initial_balance);
        Ok((addr, executor, logs))
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        &self,
        filter: &impl TestFilter,
        fuzzer: Option<TestRunner>,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
    ) -> Result<BTreeMap<String, TestResult>> {
        tracing::info!("starting tests");
        let start = Instant::now();
        let needs_setup = self.contract.functions().any(|func| func.name == "setUp");
        let test_fns = self
            .contract
            .functions()
            .into_iter()
            .filter(|func| func.name.starts_with("test"))
            .filter(|func| filter.matches_test(&func.name))
            .collect::<Vec<_>>();

        // run all unit tests
        let unit_tests = test_fns
            .par_iter()
            .filter(|func| func.inputs.is_empty())
            .map(|func| {
                let result = self.run_test(func, needs_setup, known_contracts)?;
                Ok((func.signature(), result))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        let map = if let Some(fuzzer) = fuzzer {
            let fuzz_tests = test_fns
                .par_iter()
                .filter(|func| !func.inputs.is_empty())
                .map(|func| {
                    let result =
                        self.run_fuzz_test(func, needs_setup, fuzzer.clone(), known_contracts)?;
                    Ok((func.signature(), result))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;

            let mut map = unit_tests;
            map.extend(fuzz_tests);
            map
        } else {
            unit_tests
        };

        if !map.is_empty() {
            let successful = map.iter().filter(|(_, tst)| tst.success).count();
            let duration = Instant::now().duration_since(start);
            tracing::info!(?duration, "done. {}/{} successful", successful, map.len());
        }
        Ok(map)
    }

    #[tracing::instrument(name = "test", skip_all, fields(name = %func.signature()))]
    pub fn run_test(
        &self,
        func: &Function,
        setup: bool,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
    ) -> Result<TestResult> {
        let start = Instant::now();
        // the expected result depends on the function name
        // DAppTools' ds-test will not revert inside its `assertEq`-like functions
        // which allows to test multiple assertions in 1 test function while also
        // preserving logs.
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "unit-testing");

        let (address, mut evm, init_logs) = self.new_sputnik_evm()?;

        let errors_abi = self.execution_info.as_ref().map(|(_, _, errors)| errors);
        let errors_abi = if let Some(ref abi) = errors_abi { abi } else { self.contract };

        let mut logs = init_logs;

        let mut traces: Option<Vec<CallTraceArena>> = None;
        let mut identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

        // clear out the deployment trace
        evm.reset_traces();

        // call the setup function in each test to reset the test's state.
        if setup {
            tracing::trace!("setting up");
            let setup_logs = match evm.setup(address) {
                Ok((_reason, setup_logs)) => setup_logs,
                Err(e) => {
                    // if tracing is enabled, just return it as a failed test
                    // otherwise abort
                    if evm.tracing_enabled() {
                        self.update_traces(
                            &mut traces,
                            &mut identified_contracts,
                            known_contracts,
                            setup,
                            &mut evm,
                        );
                    }

                    return Ok(TestResult {
                        success: false,
                        reason: Some("Setup failed: ".to_string() + &e.to_string()),
                        gas_used: 0,
                        counterexample: None,
                        logs,
                        kind: TestKind::Standard(0),
                        traces,
                        identified_contracts,
                        debug_calls: if evm.state().debug_enabled {
                            Some(evm.debug_calls())
                        } else {
                            None
                        },
                        labeled_addresses: evm.state().labels.clone(),
                    })
                }
            };
            logs.extend_from_slice(&setup_logs);
        }

        let (status, reason, gas_used, logs) = match evm.call::<(), _, _>(
            self.sender,
            address,
            func.clone(),
            (),
            0.into(),
            Some(errors_abi),
        ) {
            Ok((_, status, gas_used, execution_logs)) => {
                logs.extend(execution_logs);
                (status, None, gas_used, logs)
            }
            Err(err) => match err {
                EvmError::Execution { reason, gas_used, logs: execution_logs } => {
                    logs.extend(execution_logs);
                    // add reverted logs
                    logs.extend(evm.all_logs());
                    (revert(&evm), Some(reason), gas_used, logs)
                }
                err => {
                    tracing::error!(?err);
                    return Err(err.into())
                }
            },
        };

        self.update_traces(
            &mut traces,
            &mut identified_contracts,
            known_contracts,
            setup,
            &mut evm,
        );

        let success = evm.check_success(address, &status, should_fail);
        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success, %gas_used);

        Ok(TestResult {
            success,
            reason,
            gas_used,
            counterexample: None,
            logs,
            kind: TestKind::Standard(gas_used),
            traces,
            identified_contracts,
            debug_calls: if evm.state().debug_enabled { Some(evm.debug_calls()) } else { None },
            labeled_addresses: evm.state().labels.clone(),
        })
    }

    #[tracing::instrument(name = "fuzz-test", skip_all, fields(name = %func.signature()))]
    pub fn run_fuzz_test(
        &self,
        func: &Function,
        setup: bool,
        runner: TestRunner,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
    ) -> Result<TestResult> {
        // do not trace in fuzztests, as it's a big performance hit
        let start = Instant::now();
        let should_fail = func.name.starts_with("testFail");
        tracing::debug!(func = ?func.signature(), should_fail, "fuzzing");

        let (address, mut evm, init_logs) = self.new_sputnik_evm()?;

        let mut traces: Option<Vec<CallTraceArena>> = None;
        let mut identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

        // clear out the deployment trace
        evm.reset_traces();

        // call the setup function in each test to reset the test's state.
        if setup {
            tracing::trace!("setting up");
            match evm.setup(address) {
                Ok((_reason, _setup_logs)) => {}
                Err(e) => {
                    // if tracing is enabled, just return it as a failed test
                    // otherwise abort
                    if evm.tracing_enabled() {
                        self.update_traces(
                            &mut traces,
                            &mut identified_contracts,
                            known_contracts,
                            setup,
                            &mut evm,
                        );
                    }
                    return Ok(TestResult {
                        success: false,
                        reason: Some("Setup failed: ".to_string() + &e.to_string()),
                        gas_used: 0,
                        counterexample: None,
                        logs: vec![],
                        kind: TestKind::Fuzz(FuzzedCases::new(vec![])),
                        traces,
                        identified_contracts,
                        debug_calls: if evm.state().debug_enabled {
                            Some(evm.debug_calls())
                        } else {
                            None
                        },
                        labeled_addresses: evm.state().labels.clone(),
                    })
                }
            }
        }

        let mut logs = init_logs;

        let prev = evm.set_tracing_enabled(false);

        // instantiate the fuzzed evm in line
        let evm = FuzzedExecutor::new(&mut evm, runner, self.sender);
        let FuzzTestResult { cases, test_error } =
            evm.fuzz(func, address, should_fail, Some(self.contract));

        let evm = evm.into_inner();
        if let Some(ref error) = test_error {
            // we want traces for a failed fuzz
            if let TestError::Fail(_reason, bytes) = &error.test_error {
                if prev {
                    let _ = evm.set_tracing_enabled(true);
                }
                let (_retdata, status, _gas, execution_logs) =
                    evm.call_raw(self.sender, address, bytes.clone(), 0.into(), false)?;
                if is_fail(evm, status) {
                    logs.extend(execution_logs);
                    // add reverted logs
                    logs.extend(evm.all_logs());
                } else {
                    logs.extend(execution_logs);
                }
                self.update_traces(
                    &mut traces,
                    &mut identified_contracts,
                    known_contracts,
                    setup,
                    evm,
                );
            }
        }

        let success = test_error.is_none();
        let mut counterexample = None;
        let mut reason = None;
        if let Some(err) = test_error {
            match err.test_error {
                TestError::Fail(_, value) => {
                    // skip the function selector when decoding
                    let args = func.decode_input(&value.as_ref()[4..])?;
                    let counter = CounterExample { calldata: value.clone(), args };
                    counterexample = Some(counter);
                    tracing::info!("Found minimal failing case: {}", hex::encode(&value));
                }
                result => panic!("Unexpected test result: {:?}", result),
            }
            if !err.revert_reason.is_empty() {
                reason = Some(err.revert_reason);
            }
        }

        let duration = Instant::now().duration_since(start);
        tracing::debug!(?duration, %success);

        // from that call?
        Ok(TestResult {
            success,
            reason,
            gas_used: cases.median_gas(),
            counterexample,
            logs,
            kind: TestKind::Fuzz(cases),
            traces,
            identified_contracts,
            debug_calls: if evm.state().debug_enabled { Some(evm.debug_calls()) } else { None },
            labeled_addresses: evm.state().labels.clone(),
        })
    }

    fn update_traces<S: Clone, E: Evm<S>>(
        &self,
        traces: &mut Option<Vec<CallTraceArena>>,
        identified_contracts: &mut Option<BTreeMap<Address, (String, Abi)>>,
        known_contracts: Option<&BTreeMap<String, (Abi, Vec<u8>)>>,
        setup: bool,
        evm: &mut E,
    ) {
        let evm_traces = evm.traces();
        if !evm_traces.is_empty() && evm.tracing_enabled() {
            let mut ident = BTreeMap::new();
            // create an iter over the traces
            let mut trace_iter = evm_traces.into_iter();
            let mut temp_traces = Vec::new();
            if setup {
                // grab the setup trace if it exists
                let setup = trace_iter.next().expect("no setup trace");
                setup.update_identified(
                    0,
                    known_contracts.expect("traces enabled but no identified_contracts"),
                    &mut ident,
                    evm,
                );
                temp_traces.push(setup);
            }
            // grab the test trace
            if let Some(test_trace) = trace_iter.next() {
                test_trace.update_identified(
                    0,
                    known_contracts.expect("traces enabled but no identified_contracts"),
                    &mut ident,
                    evm,
                );
                temp_traces.push(test_trace);
            }

            // pass back the identified contracts and traces
            *identified_contracts = Some(ident);
            *traces = Some(temp_traces);
        }
        evm.reset_traces();
    }
}

// Helper functions for getting the revert status for a `ReturnReason` without having
// to specify the full EVM signature
fn is_fail<S: Clone, E: Evm<S> + evm_adapters::Evm<S, ReturnReason = T>, T>(
    _evm: &mut E,
    status: T,
) -> bool {
    <E as evm_adapters::Evm<S>>::is_fail(&status)
}

fn revert<S: Clone, E: Evm<S> + evm_adapters::Evm<S, ReturnReason = T>, T>(_evm: &E) -> T {
    <E as evm_adapters::Evm<S>>::revert()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{Filter, BACKEND, COMPILED, EVM_OPTS};
    use ethers::solc::artifacts::CompactContractRef;

    mod sputnik {
        use ::sputnik::backend::MemoryBackend;
        use evm_adapters::sputnik::helpers::CFG_NO_LMT;
        use foundry_utils::get_func;
        use proptest::test_runner::Config as FuzzConfig;

        use super::*;

        pub fn runner<'a>(
            abi: &'a Abi,
            code: ethers::prelude::Bytes,
        ) -> ContractRunner<'a, MemoryBackend<'a>> {
            ContractRunner::new(&*EVM_OPTS, &*CFG_NO_LMT, &*BACKEND, abi, code, None, None)
        }

        #[test]
        fn test_runner() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            super::test_runner(compiled);
        }

        #[test]
        fn test_function_overriding() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");

            let (_, code, _) = compiled.into_parts_or_default();
            let runner = runner(compiled.abi.as_ref().unwrap(), code);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let results =
                runner.run_tests(&Filter::new("testGreeting", ".*"), Some(fuzzer), None).unwrap();
            assert!(results["testGreeting()"].success);
            assert!(results["testGreeting(string)"].success);
            assert!(results["testGreeting(string,string)"].success);
        }

        #[test]
        fn test_fuzzing_counterexamples() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let (_, code, _) = compiled.into_parts_or_default();
            let runner = runner(compiled.abi.as_ref().unwrap(), code);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let results =
                runner.run_tests(&Filter::new("testFuzz.*", ".*"), Some(fuzzer), None).unwrap();
            for (_, res) in results {
                assert!(!res.success);
                assert!(res.counterexample.is_some());
            }
        }

        #[test]
        fn test_fuzzing_ok() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let (_, code, _) = compiled.into_parts_or_default();
            let runner = runner(compiled.abi.as_ref().unwrap(), code);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let func = get_func("testStringFuzz(string)").unwrap();
            let res = runner.run_fuzz_test(&func, true, fuzzer, None).unwrap();
            assert!(res.success);
            assert!(res.counterexample.is_none());
        }

        #[test]
        fn test_fuzz_shrinking() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            let (_, code, _) = compiled.into_parts_or_default();
            let runner = runner(compiled.abi.as_ref().unwrap(), code);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let fuzzer = TestRunner::new(cfg);
            let func = get_func("function testShrinking(uint256 x, uint256 y) public").unwrap();
            let res = runner.run_fuzz_test(&func, true, fuzzer, None).unwrap();
            assert!(!res.success);

            // get the counterexample with shrinking enabled by default
            let counterexample = res.counterexample.unwrap();
            let product_with_shrinking: u64 =
                // casting to u64 here is safe because the shrunk result is always gonna be small
                // enough to fit in a u64, whereas as seen below, that's not possible without
                // shrinking
                counterexample.args.into_iter().map(|x| x.into_uint().unwrap().as_u64()).product();

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            // we reduce the shrinking iters and observe a larger result
            cfg.max_shrink_iters = 5;
            let fuzzer = TestRunner::new(cfg);
            let res = runner.run_fuzz_test(&func, true, fuzzer, None).unwrap();
            assert!(!res.success);

            // get the non-shrunk result
            let counterexample = res.counterexample.unwrap();
            let args =
                counterexample.args.into_iter().map(|x| x.into_uint().unwrap()).collect::<Vec<_>>();
            let product_without_shrinking = args[0].saturating_mul(args[1]);
            assert!(product_without_shrinking > product_with_shrinking.into());
        }
    }

    pub fn test_runner(compiled: CompactContractRef) {
        let (_, code, _) = compiled.into_parts_or_default();
        let runner = sputnik::runner(compiled.abi.as_ref().unwrap(), code);

        let res = runner.run_tests(&Filter::new(".*", ".*"), None, None).unwrap();
        assert!(!res.is_empty());
        assert!(res.iter().all(|(_, result)| result.success));
    }
}
