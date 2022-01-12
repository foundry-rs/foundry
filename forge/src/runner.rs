use crate::{EvmOpts, TestFilter};
use evm_adapters::{
    sputnik::{helpers::TestSputnikVM, Executor, PRECOMPILES_MAP},
    FAUCET_ACCOUNT,
};
use rayon::iter::ParallelIterator;
use sputnik::{
    backend::{MemoryBackend, MemoryVicinity},
    Config,
};

use ethers::{
    abi::{Abi, Function, Token},
    types::{Address, Bytes, U256},
};
use evm_adapters::call_tracing::CallTraceArena;

use evm_adapters::{
    fuzz::{FuzzTestResult, FuzzedCases, FuzzedExecutor},
    Evm, EvmError,
};
use eyre::{Context, Result};
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

pub struct ContractRunner<'a> {
    /// Mutable reference to the EVM type.
    /// This is a temporary hack to work around the mutability restrictions of
    /// [`proptest::TestRunner::run`] which takes a `Fn` preventing interior mutability. [See also](https://github.com/gakonst/dapptools-rs/pull/44).
    /// Wrapping it like that allows the `test` function to gain mutable access regardless and
    /// since we don't use any parallelized fuzzing yet the `test` function has exclusive access of
    /// the mutable reference over time of its existence.
    pub evm_opts: EvmOpts,
    /// The deployed contract's ABI
    pub contract: &'a Abi,
    /// The deployed contract's address
    pub code: ethers::prelude::Bytes,
    /// The address which will be used as the `from` field in all EVM calls
    pub sender: Address,
}

impl<'a> ContractRunner<'a> {
    pub fn new(
        evm_opts: EvmOpts,
        contract: &'a Abi,
        code: ethers::prelude::Bytes,
        sender: Option<Address>,
    ) -> Self {
        Self { evm_opts, contract, code, sender: sender.unwrap_or_default() }
    }
}

impl<'a> ContractRunner<'a> {
    pub fn new_sputnik_evm(
        &self,
        cfg: &'a mut Config,
        vicinity: &'a MemoryVicinity,
    ) -> eyre::Result<(Address, TestSputnikVM<'a, MemoryBackend<'a>>, Vec<String>)> {
        // We disable the contract size limit by default, because Solidity
        // test smart contracts are likely to be >24kb
        cfg.create_contract_limit = None;

        let mut backend = MemoryBackend::new(vicinity, Default::default());
        // max out the balance of the faucet
        let faucet = backend.state_mut().entry(*FAUCET_ACCOUNT).or_insert_with(Default::default);
        faucet.balance = U256::MAX;

        let mut executor = Executor::new_with_cheatcodes(
            backend,
            self.evm_opts.env.gas_limit,
            cfg,
            &*PRECOMPILES_MAP,
            self.evm_opts.ffi,
            self.evm_opts.verbosity > 2,
            self.evm_opts.debug,
        );

        let (addr, _, _, logs) =
            executor.deploy(self.sender, self.code.clone(), 0u32.into()).expect("couldn't deploy");
        executor.set_balance(addr, self.evm_opts.initial_balance);
        Ok((addr, executor, logs))
    }

    pub fn new_odin_evm(&self) {
        todo!("evm odin not support currently");
    }

    /// Runs all tests for a contract whose names match the provided regular expression
    pub fn run_tests(
        &mut self,
        filter: &impl TestFilter,
        fuzzer: Option<&mut TestRunner>,
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

        let mut cfg = Config::london();
        let vicinity = self.evm_opts.vicinity().unwrap();
        let (address, mut evm, init_logs) = self.new_sputnik_evm(&mut cfg, &vicinity)?;

        let mut logs = init_logs;

        // call the setup function in each test to reset the test's state.
        if setup {
            tracing::trace!("setting up");
            let setup_logs = evm
                .setup(address)
                .wrap_err(format!("could not setup during {} test", func.signature()))?
                .1;
            logs.extend_from_slice(&setup_logs);
        }

        let (status, reason, gas_used, logs) =
            match evm.call::<(), _, _>(self.sender, address, func.clone(), (), 0.into()) {
                Ok((_, status, gas_used, execution_logs)) => {
                    logs.extend(execution_logs);
                    (status, None, gas_used, logs)
                }
                Err(err) => match err {
                    EvmError::Execution { reason, gas_used, logs: execution_logs } => {
                        logs.extend(execution_logs);
                        // add reverted logs
                        logs.extend(evm.all_logs());
                        (Self::revert(&evm), Some(reason), gas_used, logs)
                    }
                    err => {
                        tracing::error!(?err);
                        return Err(err.into())
                    }
                },
            };

        let mut traces: Option<Vec<CallTraceArena>> = None;
        let mut identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

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

        let mut cfg = Config::london();
        let vicinity = self.evm_opts.vicinity().unwrap();
        let (address, mut evm, init_logs) = self.new_sputnik_evm(&mut cfg, &vicinity)?;

        // call the setup function in each test to reset the test's state.
        if setup {
            evm.setup(address)?;
        }

        let mut logs = init_logs;

        let prev = evm.set_tracing_enabled(false);

        // instantiate the fuzzed evm in line
        let evm = FuzzedExecutor::new(&mut evm, runner, self.sender);
        let FuzzTestResult { cases, test_error } = evm.fuzz(func, address, should_fail);

        let FuzzedExecutor { evm, .. } = evm;
        let evm = evm.into_inner();
        let mut traces: Option<Vec<CallTraceArena>> = None;
        let mut identified_contracts: Option<BTreeMap<Address, (String, Abi)>> = None;

        if let Some(ref error) = test_error {
            // we want traces for a failed fuzz
            if let TestError::Fail(_reason, bytes) = &error.test_error {
                if prev {
                    let _ = evm.set_tracing_enabled(true);
                }
                let (_retdata, status, _gas, execution_logs) =
                    evm.call_raw(self.sender, address, bytes.clone(), 0.into(), false)?;
                if Self::is_fail(evm, status) {
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
        })
    }

    fn is_fail<S: Clone, E: Evm<S> + evm_adapters::Evm<S, ReturnReason = T>, T>(
        _evm: &mut E,
        status: T,
    ) -> bool {
        <E as evm_adapters::Evm<S>>::is_fail(&status)
    }

    fn revert<S: Clone, E: Evm<S> + evm_adapters::Evm<S, ReturnReason = T>, T>(_evm: &E) -> T {
        <E as evm_adapters::Evm<S>>::revert()
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
            let test_trace = trace_iter.next().expect("no test trace");
            test_trace.update_identified(
                0,
                known_contracts.expect("traces enabled but no identified_contracts"),
                &mut ident,
                evm,
            );
            temp_traces.push(test_trace);

            // pass back the identified contracts and traces
            *identified_contracts = Some(ident);
            *traces = Some(temp_traces);
        }
        evm.reset_traces();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{Filter, COMPILED, EVM_OPTS};
    use ethers::solc::artifacts::CompactContractRef;

    pub fn test_runner(compiled: CompactContractRef) {
        let (_, code, _) = compiled.into_parts_or_default();

        let mut runner =
            ContractRunner::new(EVM_OPTS.clone(), compiled.abi.as_ref().unwrap(), code, None);

        let res = runner.run_tests(&Filter::new(".*", ".*"), None, None).unwrap();
        assert!(!res.is_empty());
        assert!(res.iter().all(|(_, result)| result.success));
    }

    mod sputnik {
        use foundry_utils::get_func;
        use proptest::test_runner::Config as FuzzConfig;

        use super::*;

        #[test]
        fn test_runner() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");
            super::test_runner(compiled);
        }

        #[test]
        fn test_function_overriding() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");

            let (_, code, _) = compiled.into_parts_or_default();
            let mut runner =
                ContractRunner::new(EVM_OPTS.clone(), compiled.abi.as_ref().unwrap(), code, None);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let mut fuzzer = TestRunner::new(cfg);
            let results = runner
                .run_tests(&Filter::new("testGreeting", ".*"), Some(&mut fuzzer), None)
                .unwrap();
            assert!(results["testGreeting()"].success);
            assert!(results["testGreeting(string)"].success);
            assert!(results["testGreeting(string,string)"].success);
        }

        #[test]
        fn test_fuzzing_counterexamples() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");

            let (_, code, _) = compiled.into_parts_or_default();
            let mut runner =
                ContractRunner::new(EVM_OPTS.clone(), compiled.abi.as_ref().unwrap(), code, None);

            let mut cfg = FuzzConfig::default();
            cfg.failure_persistence = None;
            let mut fuzzer = TestRunner::new(cfg);
            let results = runner
                .run_tests(&Filter::new("testFuzz.*", ".*"), Some(&mut fuzzer), None)
                .unwrap();
            for (_, res) in results {
                assert!(!res.success);
                assert!(res.counterexample.is_some());
            }
        }

        #[test]
        fn test_fuzzing_ok() {
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");

            let (_, code, _) = compiled.into_parts_or_default();
            let runner =
                ContractRunner::new(EVM_OPTS.clone(), compiled.abi.as_ref().unwrap(), code, None);

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
            let runner =
                ContractRunner::new(EVM_OPTS.clone(), compiled.abi.as_ref().unwrap(), code, None);

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

    mod evmodin_test {
        use super::*;
        use ::evmodin::{tracing::NoopTracer, util::mocked_host::MockedHost, Revision};
        use evm_adapters::evmodin::EvmOdin;

        #[test]
        #[ignore]
        fn test_runner() {
            let revision = Revision::Istanbul;
            let compiled = COMPILED.find("GreeterTest").expect("could not find contract");

            let host = MockedHost::default();

            let gas_limit = 12_000_000;
            let _evm = EvmOdin::new(host, gas_limit, revision, NoopTracer);
            super::test_runner(compiled);
        }
    }
}
