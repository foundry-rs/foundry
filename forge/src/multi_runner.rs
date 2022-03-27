use crate::{ContractRunner, TestFilter, TestResult};
use ethers::{
    abi::Abi,
    prelude::{artifacts::CompactContractBytecode, ArtifactId, ArtifactOutput},
    solc::{Artifact, ProjectCompileOutput},
    types::{Address, Bytes, U256},
};
use eyre::Result;
use foundry_evm::executor::{
    builder::Backend, opts::EvmOpts, DatabaseRef, Executor, ExecutorBuilder, Fork, SpecId,
};
use foundry_utils::PostLinkInput;
use proptest::test_runner::TestRunner;
use rayon::prelude::*;
use std::{collections::BTreeMap, marker::Sync, sync::mpsc::Sender};

/// Builder used for instantiating the multi-contract runner
#[derive(Debug, Default)]
pub struct MultiContractRunnerBuilder {
    /// The fuzzer to be used for running fuzz tests
    pub fuzzer: Option<TestRunner>,
    /// The address which will be used to deploy the initial contracts and send all
    /// transactions
    pub sender: Option<Address>,
    /// The initial balance for each one of the deployed smart contracts
    pub initial_balance: U256,
    /// The EVM spec to use
    pub evm_spec: Option<SpecId>,
    /// The fork config
    pub fork: Option<Fork>,
}

pub type DeployableContracts = BTreeMap<String, (Abi, Bytes, Vec<Bytes>)>;

impl MultiContractRunnerBuilder {
    /// Given an EVM, proceeds to return a runner which is able to execute all tests
    /// against that evm
    pub fn build<A>(
        self,
        output: ProjectCompileOutput<A>,
        evm_opts: EvmOpts,
    ) -> Result<MultiContractRunner>
    where
        A: ArtifactOutput,
    {
        // This is just the contracts compiled, but we need to merge this with the read cached
        // artifacts
        let contracts = output
            .into_artifacts()
            .map(|(i, c)| (i, c.into_contract_bytecode()))
            .collect::<Vec<(ArtifactId, CompactContractBytecode)>>();

        let source_paths = contracts
            .iter()
            .map(|(i, _)| (i.slug(), i.source.to_string_lossy().into()))
            .collect::<BTreeMap<String, String>>();

        let mut known_contracts: BTreeMap<ArtifactId, (Abi, Vec<u8>)> = Default::default();

        // create a mapping of name => (abi, deployment code, Vec<library deployment code>)
        let mut deployable_contracts = DeployableContracts::default();

        foundry_utils::link(
            BTreeMap::from_iter(contracts),
            &mut known_contracts,
            evm_opts.sender,
            &mut deployable_contracts,
            |file, key| (format!("{}.json:{}", key, key), file, key),
            |post_link_input| {
                let PostLinkInput {
                    contract,
                    known_contracts,
                    id,
                    extra: deployable_contracts,
                    dependencies,
                } = post_link_input;

                // get bytes
                let bytecode =
                    if let Some(b) = contract.bytecode.expect("No bytecode").object.into_bytes() {
                        b
                    } else {
                        return Ok(())
                    };

                let abi = contract.abi.expect("We should have an abi by now");
                // if its a test, add it to deployable contracts
                if abi.constructor.as_ref().map(|c| c.inputs.is_empty()).unwrap_or(true) &&
                    abi.functions().any(|func| func.name.starts_with("test"))
                {
                    deployable_contracts
                        .insert(id.slug(), (abi.clone(), bytecode, dependencies.to_vec()));
                }

                contract
                    .deployed_bytecode
                    .and_then(|d_bcode| d_bcode.bytecode)
                    .and_then(|bcode| bcode.object.into_bytes())
                    .and_then(|bytes| known_contracts.insert(id, (abi, bytes.to_vec())));
                Ok(())
            },
        )?;

        let execution_info = foundry_utils::flatten_known_contracts(&known_contracts);
        Ok(MultiContractRunner {
            contracts: deployable_contracts,
            known_contracts,
            evm_opts,
            evm_spec: self.evm_spec.unwrap_or(SpecId::LONDON),
            sender: self.sender,
            fuzzer: self.fuzzer,
            errors: Some(execution_info.2),
            source_paths,
            fork: self.fork,
        })
    }

    #[must_use]
    pub fn sender(mut self, sender: Address) -> Self {
        self.sender = Some(sender);
        self
    }

    #[must_use]
    pub fn initial_balance(mut self, initial_balance: U256) -> Self {
        self.initial_balance = initial_balance;
        self
    }

    #[must_use]
    pub fn fuzzer(mut self, fuzzer: TestRunner) -> Self {
        self.fuzzer = Some(fuzzer);
        self
    }

    #[must_use]
    pub fn evm_spec(mut self, spec: SpecId) -> Self {
        self.evm_spec = Some(spec);
        self
    }

    #[must_use]
    pub fn with_fork(mut self, fork: Option<Fork>) -> Self {
        self.fork = fork;
        self
    }
}

/// A multi contract runner receives a set of contracts deployed in an EVM instance and proceeds
/// to run all test functions in these contracts.
pub struct MultiContractRunner {
    /// Mapping of contract name to Abi, creation bytecode and library bytecode which
    /// needs to be deployed & linked against
    pub contracts: BTreeMap<String, (Abi, ethers::prelude::Bytes, Vec<ethers::prelude::Bytes>)>,
    /// Compiled contracts by name that have an Abi and runtime bytecode
    pub known_contracts: BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    /// The EVM instance used in the test runner
    pub evm_opts: EvmOpts,
    /// The EVM spec
    pub evm_spec: SpecId,
    /// All known errors, used for decoding reverts
    pub errors: Option<Abi>,
    /// The fuzzer which will be used to run parametric tests (w/ non-0 solidity args)
    fuzzer: Option<TestRunner>,
    /// The address which will be used as the `from` field in all EVM calls
    sender: Option<Address>,
    /// A map of contract names to absolute source file paths
    pub source_paths: BTreeMap<String, String>,
    /// The fork config
    pub fork: Option<Fork>,
}

impl MultiContractRunner {
    pub fn count_filtered_tests(&self, filter: &(impl TestFilter + Send + Sync)) -> usize {
        self.contracts
            .iter()
            .filter(|(name, _)| {
                filter.matches_path(&self.source_paths.get(*name).unwrap()) &&
                    filter.matches_contract(name)
            })
            .flat_map(|(_, (abi, _, _))| {
                abi.functions().filter(|func| filter.matches_test(&func.name))
            })
            .count()
    }

    pub fn test(
        &mut self,
        filter: &(impl TestFilter + Send + Sync),
        stream_result: Option<Sender<(String, BTreeMap<String, TestResult>)>>,
    ) -> Result<BTreeMap<String, BTreeMap<String, TestResult>>> {
        let env = self.evm_opts.evm_env();

        // the db backend that serves all the data
        let db = Backend::new(self.fork.take(), &env);

        let results = self
            .contracts
            .par_iter()
            .filter(|(name, _)| {
                filter.matches_path(&self.source_paths.get(*name).unwrap()) &&
                    filter.matches_contract(name)
            })
            .filter(|(_, (abi, _, _))| abi.functions().any(|func| filter.matches_test(&func.name)))
            .map(|(name, (abi, deploy_code, libs))| {
                let mut builder = ExecutorBuilder::new()
                    .with_cheatcodes(self.evm_opts.ffi)
                    .with_config(env.clone())
                    .with_spec(self.evm_spec)
                    .with_gas_limit(self.evm_opts.gas_limit());

                if self.evm_opts.verbosity >= 3 {
                    builder = builder.with_tracing();
                }

                let executor = builder.build(db.clone());
                let result =
                    self.run_tests(name, abi, executor, deploy_code.clone(), libs, filter)?;
                Ok((name.clone(), result))
            })
            .filter_map(Result::<_>::ok)
            .filter(|(_, results)| !results.is_empty())
            .map_with(stream_result, |stream_result, (name, result)| {
                if let Some(stream_result) = stream_result.as_ref() {
                    stream_result.send((name.clone(), result.clone())).unwrap();
                }
                (name, result)
            })
            .collect::<BTreeMap<_, _>>();
        Ok(results)
    }

    // The _name field is unused because we only want it for tracing
    #[tracing::instrument(
        name = "contract",
        skip_all,
        err,
        fields(name = %_name)
    )]
    fn run_tests<DB: DatabaseRef + Send + Sync>(
        &self,
        _name: &str,
        contract: &Abi,
        executor: Executor<DB>,
        deploy_code: Bytes,
        libs: &[Bytes],
        filter: &impl TestFilter,
    ) -> Result<BTreeMap<String, TestResult>> {
        let mut runner = ContractRunner::new(
            executor,
            contract,
            deploy_code,
            self.evm_opts.initial_balance,
            self.sender,
            self.errors.as_ref(),
            libs,
        );
        runner.run_tests(filter, self.fuzzer.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decode::decode_console_logs,
        test_helpers::{filter::Filter, COMPILED, EVM_OPTS},
    };
    use foundry_evm::trace::TraceKind;

    /// Builds a base runner
    fn base_runner() -> MultiContractRunnerBuilder {
        MultiContractRunnerBuilder::default().sender(EVM_OPTS.sender)
    }

    /// Builds a non-tracing runner
    fn runner() -> MultiContractRunner {
        base_runner().build((*COMPILED).clone(), EVM_OPTS.clone()).unwrap()
    }

    /// Builds a tracing runner
    fn tracing_runner() -> MultiContractRunner {
        let mut opts = EVM_OPTS.clone();
        opts.verbosity = 5;
        base_runner().build((*COMPILED).clone(), opts).unwrap()
    }

    /// A helper to assert the outcome of multiple tests with helpful assert messages
    fn assert_multiple(
        actuals: &BTreeMap<String, BTreeMap<String, TestResult>>,
        expecteds: BTreeMap<&str, Vec<(&str, bool, Option<String>, Option<Vec<String>>)>>,
    ) {
        assert_eq!(
            actuals.len(),
            expecteds.len(),
            "We did not run as many contracts as we expected"
        );
        for (contract_name, tests) in &expecteds {
            assert_eq!(
                actuals[*contract_name].len(),
                expecteds[contract_name].len(),
                "We did not run as many test functions as we expected for {}",
                contract_name
            );
            for (test_name, should_pass, reason, expected_logs) in tests {
                let logs = decode_console_logs(&actuals[*contract_name][*test_name].logs);

                if *should_pass {
                    assert!(
                        actuals[*contract_name][*test_name].success,
                        "Test {} did not pass as expected.\nReason: {:?}\nLogs:\n{}",
                        test_name,
                        actuals[*contract_name][*test_name].reason,
                        logs.join("\n")
                    );
                } else {
                    assert!(
                        !actuals[*contract_name][*test_name].success,
                        "Test {} did not fail as expected.\nLogs:\n{}",
                        test_name,
                        logs.join("\n")
                    );
                    assert_eq!(
                        actuals[*contract_name][*test_name].reason, *reason,
                        "Failure reason for test {} did not match what we expected.",
                        test_name
                    );
                }

                if let Some(expected_logs) = expected_logs {
                    assert!(
                        logs.iter().eq(expected_logs.iter()),
                        "Logs did not match for test {}.\nExpected:\n{}\n\nGot:\n{}",
                        test_name,
                        logs.join("\n"),
                        expected_logs.join("\n")
                    );
                }
            }
        }
    }

    #[test]
    fn test_core() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*", ".*core"), None).unwrap();

        assert_multiple(
            &results,
            BTreeMap::from([
                (
                    "FailingSetupTest.json:FailingSetupTest",
                    vec![(
                        "setUp()",
                        false,
                        Some("Setup failed: setup failed predictably".to_string()),
                        None,
                    )],
                ),
                ("RevertingTest.json:RevertingTest", vec![("testFailRevert()", true, None, None)]),
                (
                    "SetupConsistencyCheck.json:SetupConsistencyCheck",
                    vec![("testAdd()", true, None, None), ("testMultiply()", true, None, None)],
                ),
                (
                    "DSStyleTest.json:DSStyleTest",
                    vec![
                        ("testAddresses()", true, None, None),
                        ("testEnvironment()", true, None, None),
                    ],
                ),
                (
                    "PaymentFailureTest.json:PaymentFailureTest",
                    vec![("testCantPay()", false, Some("Revert".to_string()), None)],
                ),
                (
                    "LibraryLinkingTest.json:LibraryLinkingTest",
                    vec![("testDirect()", true, None, None), ("testNested()", true, None, None)],
                ),
                ("AbstractTest.json:AbstractTest", vec![("testSomething()", true, None, None)]),
            ]),
        );
    }

    #[test]
    fn test_logs() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*", ".*logs"), None).unwrap();

        assert_multiple(
            &results,
            BTreeMap::from([
                (
                    "DebugLogsTest.json:DebugLogsTest",
                    vec![
                        ("test1()", true, None, Some(vec!["0".into(), "1".into(), "2".into()])),
                        ("test2()", true, None, Some(vec!["0".into(), "1".into(), "3".into()])),
                        (
                            "testFailWithRequire()",
                            true,
                            None,
                            Some(vec!["0".into(), "1".into(), "5".into()]),
                        ),
                        (
                            "testFailWithRevert()",
                            true,
                            None,
                            Some(vec!["0".into(), "1".into(), "4".into(), "100".into()]),
                        ),
                    ],
                ),
                (
                    "HardhatLogsTest.json:HardhatLogsTest",
                    vec![
                        (
                            "testInts()",
                            true,
                            None,
                            Some(vec![
                                "constructor".into(),
                                "0".into(),
                                "1".into(),
                                "2".into(),
                                "3".into(),
                            ]),
                        ),
                        (
                            "testMisc()",
                            true,
                            None,
                            Some(vec![
                                "constructor".into(),
                                "testMisc, 0x0000000000000000000000000000000000000001".into(),
                                "testMisc, 42".into(),
                            ]),
                        ),
                        (
                            "testStrings()",
                            true,
                            None,
                            Some(vec!["constructor".into(), "testStrings".into()]),
                        ),
                    ],
                ),
            ]),
        );
    }

    #[test]
    fn test_cheats() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*", ".*cheats"), None).unwrap();

        for (_, tests) in results {
            for (test_name, result) in tests {
                assert!(
                    result.success,
                    "Test {} did not pass as expected.\nReason: {:?}",
                    test_name, result.reason
                );
            }
        }
    }

    #[test]
    fn test_fuzz() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*", ".*fuzz"), None).unwrap();

        for (_, tests) in results {
            for (test_name, result) in tests {
                match test_name.as_ref() {
                    "testPositive(uint256)" | "testSuccessfulFuzz(uint128,uint128)" => assert!(
                        result.success,
                        "Test {} did not pass as expected.\nReason: {:?}",
                        test_name, result.reason
                    ),
                    _ => assert!(
                        !result.success,
                        "Test {} did not fail as expected.\nReason: {:?}",
                        test_name, result.reason
                    ),
                }
            }
        }
    }

    #[test]
    fn test_trace() {
        let mut runner = tracing_runner();
        let results = runner.test(&Filter::new(".*", ".*", ".*trace"), None).unwrap();

        // TODO: This trace test is very basic - it is probably a good candidate for snapshot
        // testing.
        for (_, tests) in results {
            for (test_name, result) in tests {
                let deployment_traces = result
                    .traces
                    .iter()
                    .filter(|(kind, _)| *kind == TraceKind::Deployment)
                    .collect::<Vec<_>>();
                let setup_traces = result
                    .traces
                    .iter()
                    .filter(|(kind, _)| *kind == TraceKind::Setup)
                    .collect::<Vec<_>>();
                let execution_traces = result
                    .traces
                    .iter()
                    .filter(|(kind, _)| *kind == TraceKind::Deployment)
                    .collect::<Vec<_>>();

                assert_eq!(
                    deployment_traces.len(),
                    1,
                    "Test {} did not have exactly 1 deployment trace.",
                    test_name
                );
                assert!(setup_traces.len() <= 1, "Test {} had more than 1 setup trace.", test_name);
                assert_eq!(
                    execution_traces.len(),
                    1,
                    "Test {} did not not have exactly 1 execution trace.",
                    test_name
                );
            }
        }
    }

    #[test]
    fn test_doesnt_run_abstract_contract() {
        let mut runner = runner();
        let results = runner.test(&Filter::new(".*", ".*", ".*core/Abstract.t.sol"), None).unwrap();
        assert!(results.get("AbstractTestBase.json:AbstractTestBase").is_none());
        assert!(results.get("AbstractTest.json:AbstractTest").is_some());
    }
}
