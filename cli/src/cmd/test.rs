//! Test command

use crate::{
    cmd::{
        build::{BuildArgs, Env, EvmType},
        Cmd,
    },
    utils,
};
use ansi_term::Colour;
use ethers::{
    providers::Provider,
    solc::{ArtifactOutput, Project},
    types::{Address, U256},
};
use evm_adapters::FAUCET_ACCOUNT;
use forge::MultiContractRunnerBuilder;
use regex::Regex;
use std::{collections::BTreeMap, convert::TryFrom, sync::Arc};
use structopt::StructOpt;

#[derive(Debug, Clone, StructOpt)]
pub struct TestArgs {
    #[structopt(help = "print the test results in json format", long, short)]
    json: bool,

    #[structopt(flatten)]
    env: Env,

    #[structopt(
        long = "--match",
        short = "-m",
        help = "only run test methods matching regex",
        default_value = ".*"
    )]
    pattern: regex::Regex,

    #[structopt(flatten)]
    opts: BuildArgs,

    #[structopt(
        long,
        short,
        help = "the EVM type you want to use (e.g. sputnik, evmodin)",
        default_value = "sputnik"
    )]
    evm_type: EvmType,

    #[structopt(
        help = "fetch state over a remote instead of starting from empty state",
        long,
        short
    )]
    #[structopt(alias = "rpc-url")]
    fork_url: Option<String>,

    #[structopt(help = "pins the block number for the state fork", long)]
    #[structopt(env = "DAPP_FORK_BLOCK")]
    fork_block_number: Option<u64>,

    #[structopt(
        help = "the initial balance of each deployed test contract",
        long,
        default_value = "0xffffffffffffffffffffffff"
    )]
    initial_balance: U256,

    #[structopt(
        help = "the address which will be executing all tests",
        long,
        default_value = "0x0000000000000000000000000000000000000000",
        env = "DAPP_TEST_ADDRESS"
    )]
    sender: Address,

    #[structopt(help = "enables the FFI cheatcode", long)]
    ffi: bool,

    #[structopt(help = "verbosity of 'forge test' output (0-3)", long, default_value = "0")]
    verbosity: u8,

    #[structopt(
        help = "if set to true, the process will exit with an exit code = 0, even if the tests fail",
        long,
        env = "FORGE_ALLOW_FAILURE"
    )]
    allow_failure: bool,
}

impl Cmd for TestArgs {
    type Output = TestOutcome;

    fn run(self) -> eyre::Result<Self::Output> {
        let TestArgs {
            opts,
            env,
            json,
            pattern,
            evm_type,
            fork_url,
            fork_block_number,
            initial_balance,
            sender,
            ffi,
            verbosity,
            allow_failure,
        } = self;
        // Setup the fuzzer
        // TODO: Add CLI Options to modify the persistence
        let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
        let fuzzer = proptest::test_runner::TestRunner::new(cfg);

        // Set up the project
        let project = opts.project()?;

        // prepare the test builder
        let builder = MultiContractRunnerBuilder::default()
            .fuzzer(fuzzer)
            .initial_balance(initial_balance)
            .sender(sender);

        // run the tests depending on the chosen EVM
        match evm_type {
            #[cfg(feature = "sputnik-evm")]
            EvmType::Sputnik => {
                use evm_adapters::sputnik::{
                    vicinity, Executor, ForkMemoryBackend, PRECOMPILES_MAP,
                };
                use sputnik::backend::{Backend, MemoryBackend};
                let mut cfg = utils::sputnik_cfg(opts.evm_version);

                // We disable the contract size limit by default, because Solidity
                // test smart contracts are likely to be >24kb
                cfg.create_contract_limit = None;

                let vicinity = if let Some(ref url) = fork_url {
                    let provider = Provider::try_from(url.as_str())?;
                    let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
                    rt.block_on(vicinity(&provider, fork_block_number))?
                } else {
                    env.sputnik_state()
                };
                let mut backend = MemoryBackend::new(&vicinity, Default::default());
                // max out the balance of the faucet
                let faucet =
                    backend.state_mut().entry(*FAUCET_ACCOUNT).or_insert_with(Default::default);
                faucet.balance = U256::MAX;

                let backend: Box<dyn Backend> = if let Some(ref url) = fork_url {
                    let provider = Provider::try_from(url.as_str())?;
                    let init_state = backend.state().clone();
                    let backend =
                        ForkMemoryBackend::new(provider, backend, fork_block_number, init_state);
                    Box::new(backend)
                } else {
                    Box::new(backend)
                };
                let backend = Arc::new(backend);

                let precompiles = PRECOMPILES_MAP.clone();
                let evm = Executor::new_with_cheatcodes(
                    backend,
                    env.gas_limit,
                    &cfg,
                    &precompiles,
                    ffi,
                    verbosity > 2,
                );

                test(builder, project, evm, pattern, json, verbosity, allow_failure)
            }
            #[cfg(feature = "evmodin-evm")]
            EvmType::EvmOdin => {
                use evm_adapters::evmodin::EvmOdin;
                use evmodin::tracing::NoopTracer;

                let revision = utils::evmodin_cfg(opts.evm_version);

                // TODO: Replace this with a proper host. We'll want this to also be
                // provided generically when we add the Forking host(s).
                let host = env.evmodin_state();

                let evm = EvmOdin::new(host, env.gas_limit, revision, NoopTracer);
                test(builder, project, evm, pattern, json, verbosity, allow_failure)
            }
        }
    }
}

/// The result of a single test
#[derive(Debug, Clone)]
pub struct Test {
    /// The signature of the test
    pub signature: String,
    /// Result of the executed solidity test
    pub result: forge::TestResult,
}

impl Test {
    pub fn gas_used(&self) -> u64 {
        self.result.gas_used
    }
}

/// Represents the bundled results of all tests
pub struct TestOutcome {
    /// Whether failures are allowed
    allow_failure: bool,
    /// All test results `contract -> (test name -> TestResult)`
    pub results: BTreeMap<String, BTreeMap<String, forge::TestResult>>,
}

impl TestOutcome {
    fn new(
        results: BTreeMap<String, BTreeMap<String, forge::TestResult>>,
        allow_failure: bool,
    ) -> Self {
        Self { results, allow_failure }
    }

    /// Iterator over all succeeding tests and their names
    pub fn successes(&self) -> impl Iterator<Item = (&String, &forge::TestResult)> {
        self.tests().filter(|(_, t)| t.success)
    }

    /// Iterator over all failing tests and their names
    pub fn failures(&self) -> impl Iterator<Item = (&String, &forge::TestResult)> {
        self.tests().filter(|(_, t)| !t.success)
    }

    /// Iterator over all tests and their names
    pub fn tests(&self) -> impl Iterator<Item = (&String, &forge::TestResult)> {
        self.results.values().flat_map(|tests| tests.iter())
    }

    pub fn into_tests(self) -> impl Iterator<Item = Test> {
        self.results
            .into_values()
            .flat_map(|tests| tests.into_iter())
            .map(|(name, result)| Test { signature: name, result })
    }

    /// Checks if there are any failures and failures are disallowed
    pub fn ensure_ok(&self) -> eyre::Result<()> {
        if !self.allow_failure {
            let failures = self.failures().count();
            if failures > 0 {
                let successes = self.successes().count();
                eyre::bail!(
                    "Encountered a total of {} failing tests, {} tests succeeded",
                    failures,
                    successes
                );
            }
        }
        Ok(())
    }
}

/// Runs all the tests
fn test<A: ArtifactOutput + 'static, S: Clone, E: evm_adapters::Evm<S>>(
    builder: MultiContractRunnerBuilder,
    project: Project<A>,
    evm: E,
    pattern: Regex,
    json: bool,
    verbosity: u8,
    allow_failure: bool,
) -> eyre::Result<TestOutcome> {
    let mut runner = builder.build(project, evm)?;

    let results = runner.test(pattern)?;

    if json {
        let res = serde_json::to_string(&results)?;
        println!("{}", res);
    } else {
        // Dapptools-style printing of test results
        for (i, (contract_name, tests)) in results.iter().enumerate() {
            if i > 0 {
                println!()
            }
            if !tests.is_empty() {
                let term = if tests.len() > 1 { "tests" } else { "test" };
                println!("Running {} {} for {}", tests.len(), term, contract_name);
            }

            for (name, result) in tests {
                let status = if result.success {
                    Colour::Green.paint("[PASS]")
                } else {
                    let txt = match (&result.reason, &result.counterexample) {
                        (Some(ref reason), Some(ref counterexample)) => {
                            format!(
                                "[FAIL. Reason: {}. Counterexample: {}]",
                                reason, counterexample
                            )
                        }
                        (None, Some(ref counterexample)) => {
                            format!("[FAIL. Counterexample: {}]", counterexample)
                        }
                        (Some(ref reason), None) => {
                            format!("[FAIL. Reason: {}]", reason)
                        }
                        (None, None) => "[FAIL]".to_string(),
                    };

                    Colour::Red.paint(txt)
                };

                println!("{} {} {}", status, name, result.kind.gas_used());
            }

            if verbosity > 1 {
                println!();

                for (name, result) in tests {
                    let status = if result.success { "Success" } else { "Failure" };
                    println!("{}: {}", status, name);
                    println!();

                    for log in &result.logs {
                        println!("  {}", log);
                    }

                    println!();

                    if verbosity > 2 {
                        if let (Some(traces), Some(identified_contracts)) =
                            (&result.traces, &result.identified_contracts)
                        {
                            let mut ident = identified_contracts.clone();
                            if verbosity > 3 {
                                // print setup calls as well
                                traces.iter().for_each(|trace| {
                                    trace.pretty_print(
                                        0,
                                        &runner.known_contracts,
                                        &mut ident,
                                        &runner.evm,
                                        "",
                                    );
                                });
                            } else if !traces.is_empty() {
                                traces.last().expect("no last but not empty").pretty_print(
                                    0,
                                    &runner.known_contracts,
                                    &mut ident,
                                    &runner.evm,
                                    "",
                                );
                            }

                            println!();
                        }
                    }
                }
            }
        }
    }

    Ok(TestOutcome::new(results, allow_failure))
}
