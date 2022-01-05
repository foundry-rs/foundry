//! Test command

use crate::{
    cmd::{
        build::{BuildArgs, EvmType},
        Cmd,
    },
    opts::forge::EvmOpts,
    utils,
};
use ansi_term::Colour;
use ethers::solc::{ArtifactOutput, Project};
use forge::MultiContractRunnerBuilder;
use regex::Regex;
use std::collections::BTreeMap;
use structopt::StructOpt;

#[derive(Debug, Clone, StructOpt)]
pub struct TestArgs {
    #[structopt(help = "print the test results in json format", long, short)]
    json: bool,

    #[structopt(flatten)]
    evm_opts: EvmOpts,

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
        help = "if set to true, the process will exit with an exit code = 0, even if the tests fail",
        long,
        env = "FORGE_ALLOW_FAILURE"
    )]
    allow_failure: bool,
}

impl Cmd for TestArgs {
    type Output = TestOutcome;

    fn run(self) -> eyre::Result<Self::Output> {
        let TestArgs { opts, evm_opts, json, pattern, allow_failure } = self;
        // Setup the fuzzer
        // TODO: Add CLI Options to modify the persistence
        let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
        let fuzzer = proptest::test_runner::TestRunner::new(cfg);

        // Set up the project
        let project = opts.project()?;

        // prepare the test builder
        let builder = MultiContractRunnerBuilder::default()
            .fuzzer(fuzzer)
            .initial_balance(evm_opts.initial_balance)
            .sender(evm_opts.sender);

        // run the tests depending on the chosen EVM
        match evm_opts.evm_type {
            #[cfg(feature = "sputnik-evm")]
            EvmType::Sputnik => {
                let mut cfg = utils::sputnik_cfg(opts.compiler.evm_version);
                let vicinity = evm_opts.vicinity()?;
                let evm = utils::sputnik_helpers::evm(&evm_opts, &mut cfg, &vicinity)?;
                test(builder, project, evm, pattern, json, evm_opts.verbosity, allow_failure)
            }
            #[cfg(feature = "evmodin-evm")]
            EvmType::EvmOdin => {
                use evm_adapters::evmodin::EvmOdin;
                use evmodin::tracing::NoopTracer;

                let revision = utils::evmodin_cfg(opts.compiler.evm_version);

                // TODO: Replace this with a proper host. We'll want this to also be
                // provided generically when we add the Forking host(s).
                let host = evm_opts.env.evmodin_state();

                let evm = EvmOdin::new(host, evm_opts.env.gas_limit, revision, NoopTracer);
                test(builder, project, evm, pattern, json, evm_opts.verbosity, allow_failure)
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
                            if !result.success && verbosity == 3 || verbosity > 3 {
                                let mut ident = identified_contracts.clone();
                                if verbosity > 4 || !result.success {
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
