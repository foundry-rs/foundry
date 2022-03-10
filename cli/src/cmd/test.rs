//! Test command

use crate::{
    cmd::{build::BuildArgs, Cmd},
    opts::evm::EvmArgs,
};
use ansi_term::Colour;
use clap::{AppSettings, Parser};
use ethers::solc::{ArtifactOutput, ProjectCompileOutput};
use evm_adapters::{
    call_tracing::ExecutionInfo, evm_opts::EvmOpts, gas_report::GasReport, sputnik::helpers::vm,
};
use forge::{MultiContractRunnerBuilder, TestFilter, TestResult};
use foundry_config::{figment::Figment, Config};
use regex::Regex;
use std::{collections::BTreeMap, str::FromStr, sync::mpsc::channel, thread};

#[derive(Debug, Clone, Parser)]
pub struct Filter {
    #[clap(
        long = "match",
        short = 'm',
        help = "only run test methods matching regex (deprecated, see --match-test)"
    )]
    pub pattern: Option<regex::Regex>,

    #[clap(
        long = "match-test",
        alias = "mt",
        help = "only run test methods matching regex",
        conflicts_with = "pattern"
    )]
    pub test_pattern: Option<regex::Regex>,

    #[clap(
        long = "no-match-test",
        alias = "nmt",
        help = "only run test methods not matching regex",
        conflicts_with = "pattern"
    )]
    pub test_pattern_inverse: Option<regex::Regex>,

    #[clap(
        long = "match-contract",
        alias = "mc",
        help = "only run test methods in contracts matching regex",
        conflicts_with = "pattern"
    )]
    pub contract_pattern: Option<regex::Regex>,

    #[clap(
        long = "no-match-contract",
        alias = "nmc",
        help = "only run test methods in contracts not matching regex",
        conflicts_with = "pattern"
    )]
    contract_pattern_inverse: Option<regex::Regex>,

    #[clap(
        long = "match-path",
        alias = "mp",
        help = "only run test methods in source files at path matching regex. Requires absolute path",
        conflicts_with = "pattern"
    )]
    pub path_pattern: Option<regex::Regex>,

    #[clap(
        long = "no-match-path",
        alias = "nmp",
        help = "only run test methods in source files at path not matching regex. Requires absolute path",
        conflicts_with = "pattern"
    )]
    pub path_pattern_inverse: Option<regex::Regex>,
}

impl TestFilter for Filter {
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let test_name = test_name.as_ref();
        // Handle the deprecated option match
        if let Some(re) = &self.pattern {
            ok &= re.is_match(test_name);
        }
        if let Some(re) = &self.test_pattern {
            ok &= re.is_match(test_name);
        }
        if let Some(re) = &self.test_pattern_inverse {
            ok &= !re.is_match(test_name);
        }
        ok
    }

    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool {
        let mut ok = true;
        let contract_name = contract_name.as_ref();
        if let Some(re) = &self.contract_pattern {
            ok &= re.is_match(contract_name);
        }
        if let Some(re) = &self.contract_pattern_inverse {
            ok &= !re.is_match(contract_name);
        }
        ok
    }

    fn matches_path(&self, path: impl AsRef<str>) -> bool {
        let mut ok = true;
        let path = path.as_ref();
        if let Some(re) = &self.path_pattern {
            let re = Regex::from_str(&format!("^{}", re.as_str())).unwrap();
            ok &= re.is_match(path);
        }
        if let Some(re) = &self.path_pattern_inverse {
            let re = Regex::from_str(&format!("^{}", re.as_str())).unwrap();
            ok &= !re.is_match(path);
        }
        ok
    }
}

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(TestArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
// This is required to group Filter options in help output
#[clap(global_setting = AppSettings::DeriveDisplayOrder)]
pub struct TestArgs {
    #[clap(help = "print the test results in json format", long, short)]
    json: bool,

    #[clap(help = "print a gas report", long = "gas-report")]
    gas_report: bool,

    #[clap(flatten)]
    evm_opts: EvmArgs,

    #[clap(flatten)]
    filter: Filter,

    #[clap(flatten)]
    opts: BuildArgs,

    #[clap(
        help = "if set to true, the process will exit with an exit code = 0, even if the tests fail",
        long,
        env = "FORGE_ALLOW_FAILURE"
    )]
    allow_failure: bool,
}

impl TestArgs {
    /// Returns the flattened [`BuildArgs`]
    pub fn build_args(&self) -> &BuildArgs {
        &self.opts
    }

    /// Returns the flattened [`Filter`] arguments
    pub fn filter(&self) -> &Filter {
        &self.filter
    }
}

impl Cmd for TestArgs {
    type Output = TestOutcome;

    fn run(self) -> eyre::Result<Self::Output> {
        // merge all configs
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();

        let TestArgs { json, filter, allow_failure, .. } = self;

        // Setup the fuzzer
        // TODO: Add CLI Options to modify the persistence
        let cfg = proptest::test_runner::Config {
            failure_persistence: None,
            cases: config.fuzz_runs,
            max_local_rejects: config.fuzz_max_local_rejects,
            max_global_rejects: config.fuzz_max_global_rejects,
            ..Default::default()
        };
        let fuzzer = proptest::test_runner::TestRunner::new(cfg);

        // Set up the project
        let project = config.project()?;
        let output = super::compile(&project, false, false)?;

        // prepare the test builder
        let mut evm_cfg = crate::utils::sputnik_cfg(&config.evm_version);
        evm_cfg.create_contract_limit = None;

        let builder = MultiContractRunnerBuilder::default()
            .fuzzer(fuzzer)
            .initial_balance(evm_opts.initial_balance)
            .evm_cfg(evm_cfg)
            .sender(evm_opts.sender);

        test(
            builder,
            output,
            evm_opts,
            filter,
            json,
            allow_failure,
            (self.gas_report, config.gas_reports),
        )
    }
}

/// The result of a single test
#[derive(Debug, Clone)]
pub struct Test {
    /// The identifier of the artifact/contract in the form of `<artifact file name>:<contract
    /// name>`
    pub artifact_id: String,
    /// The signature of the solidity test
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

    /// Returns an iterator over all `Test`
    pub fn into_tests(self) -> impl Iterator<Item = Test> {
        self.results
            .into_iter()
            .flat_map(|(file, tests)| tests.into_iter().map(move |t| (file.clone(), t)))
            .map(|(artifact_id, (signature, result))| Test { artifact_id, signature, result })
    }

    /// Checks if there are any failures and failures are disallowed
    pub fn ensure_ok(&self) -> eyre::Result<()> {
        if !self.allow_failure {
            let failures = self.failures().count();
            if failures > 0 {
                println!();
                println!("Failed tests:");
                for (name, result) in self.failures() {
                    short_test_result(name, result);
                }
                println!();

                let successes = self.successes().count();
                println!(
                    "Encountered a total of {} failing tests, {} tests succeeded",
                    Colour::Red.paint(failures.to_string()),
                    Colour::Green.paint(successes.to_string())
                );
                std::process::exit(1);
            }
        }
        Ok(())
    }
}

fn short_test_result(name: &str, result: &forge::TestResult) {
    let status = if result.success {
        Colour::Green.paint("[PASS]")
    } else {
        let txt = match (&result.reason, &result.counterexample) {
            (Some(ref reason), Some(ref counterexample)) => {
                format!("[FAIL. Reason: {}. Counterexample: {}]", reason, counterexample)
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

/// Runs all the tests
fn test<A: ArtifactOutput + 'static>(
    builder: MultiContractRunnerBuilder,
    output: ProjectCompileOutput<A>,
    mut evm_opts: EvmOpts,
    filter: Filter,
    json: bool,
    allow_failure: bool,
    gas_reports: (bool, Vec<String>),
) -> eyre::Result<TestOutcome> {
    let verbosity = evm_opts.verbosity;
    let gas_reporting = gas_reports.0;
    if gas_reporting && evm_opts.verbosity < 3 {
        // force evm to do tracing, but don't hit the verbosity print path
        evm_opts.verbosity = 3;
    }
    let mut runner = builder.build(output, evm_opts)?;

    if json {
        let results = runner.test(&filter, None)?;
        let res = serde_json::to_string(&results)?; // TODO: Make this work normally
        println!("{}", res);
        Ok(TestOutcome::new(results, allow_failure))
    } else {
        // Dapptools-style printing of test results
        let mut gas_report = GasReport::new(gas_reports.1);
        let (tx, rx) = channel::<(String, BTreeMap<String, TestResult>)>();
        let known_contracts = runner.known_contracts.clone();
        let execution_info = runner.execution_info.clone();

        let handle = thread::spawn(move || {
            while let Ok((contract_name, tests)) = rx.recv() {
                println!();
                if !tests.is_empty() {
                    let term = if tests.len() > 1 { "tests" } else { "test" };
                    println!("Running {} {} for {}", tests.len(), term, contract_name);
                }
                for (name, result) in tests {
                    short_test_result(&name, &result);
                    // adds a linebreak only if there were any traces or logs, so that the
                    // output does not look like 1 big block.
                    let mut add_newline = false;
                    if verbosity > 1 && !result.logs.is_empty() {
                        add_newline = true;
                        println!("Logs:");
                        for log in &result.logs {
                            println!("  {}", log);
                        }
                    }
                    if verbosity > 2 {
                        if let (Some(traces), Some(identified_contracts)) =
                            (&result.traces, &result.identified_contracts)
                        {
                            if !result.success && verbosity == 3 || verbosity > 3 {
                                // add a new line if any logs were printed & to separate them from
                                // the traces to be printed
                                if !result.logs.is_empty() {
                                    println!();
                                }
                                let mut ident = identified_contracts.clone();
                                let (funcs, events, errors) = &execution_info;
                                let mut exec_info = ExecutionInfo::new(
                                    // &runner.known_contracts,
                                    &known_contracts,
                                    &mut ident,
                                    &result.labeled_addresses,
                                    funcs,
                                    events,
                                    errors,
                                );
                                let vm = vm();
                                let mut trace_string = "".to_string();
                                if verbosity > 4 || !result.success {
                                    add_newline = true;
                                    println!("Traces:");
                                    // print setup calls as well
                                    traces.iter().for_each(|trace| {
                                        trace.construct_trace_string(
                                            0,
                                            &mut exec_info,
                                            &vm,
                                            "  ",
                                            &mut trace_string,
                                        );
                                    });
                                } else if !traces.is_empty() {
                                    add_newline = true;
                                    println!("Traces:");
                                    traces
                                        .last()
                                        .expect("no last but not empty")
                                        .construct_trace_string(
                                            0,
                                            &mut exec_info,
                                            &vm,
                                            "  ",
                                            &mut trace_string,
                                        );
                                }
                                if !trace_string.is_empty() {
                                    println!("{}", trace_string);
                                }
                            }
                        }
                    }
                    if add_newline {
                        println!();
                    }
                }
            }
        });

        let results = runner.test(&filter, Some(tx))?;

        handle.join().unwrap();

        if gas_reporting {
            for tests in results.values() {
                for result in tests.values() {
                    if let (Some(traces), Some(identified_contracts)) =
                        (&result.traces, &result.identified_contracts)
                    {
                        gas_report.analyze(traces, identified_contracts);
                    }
                }
            }
            gas_report.finalize();
            println!("{}", gas_report);
        }
        Ok(TestOutcome::new(results, allow_failure))
    }
}
