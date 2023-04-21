//! Test command
use crate::{
    cmd::{
        forge::{build::CoreBuildArgs, debug::DebugArgs, install, watch::WatchArgs},
        Cmd, LoadConfig,
    },
    suggestions, utils,
};
use cast::fuzz::CounterExample;
use clap::Parser;
use ethers::{solc::utils::RuntimeOrHandle, types::U256};
use forge::{
    decode::decode_console_logs,
    executor::inspector::CheatsConfig,
    gas_report::GasReport,
    result::{SuiteResult, TestKind, TestResult},
    trace::{
        identifier::{EtherscanIdentifier, LocalTraceIdentifier, SignaturesIdentifier},
        CallTraceDecoderBuilder, TraceKind,
    },
    MultiContractRunner, MultiContractRunnerBuilder, TestOptions,
};
use foundry_common::{
    compile::{self, ProjectCompiler},
    evm::EvmArgs,
    get_contract_name, get_file_name,
};
use foundry_config::{figment, Config};
use regex::Regex;
use std::{collections::BTreeMap, path::PathBuf, sync::mpsc::channel, thread, time::Duration};
use tracing::trace;
use watchexec::config::{InitConfig, RuntimeConfig};
use yansi::Paint;
mod filter;
use crate::cmd::forge::test::filter::ProjectPathsAwareFilter;
pub use filter::FilterArgs;
use foundry_common::shell;
use foundry_config::figment::{
    value::{Dict, Map},
    Metadata, Profile, Provider,
};

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(TestArgs, opts, evm_opts);

/// CLI arguments for `forge test`.
#[derive(Debug, Clone, Parser)]
#[clap(next_help_heading = "Test options")]
pub struct TestArgs {
    #[clap(flatten)]
    filter: FilterArgs,

    /// Run a test in the debugger.
    ///
    /// The argument passed to this flag is the name of the test function you want to run, and it
    /// works the same as --match-test.
    ///
    /// If more than one test matches your specified criteria, you must add additional filters
    /// until only one test is found (see --match-contract and --match-path).
    ///
    /// The matching test will be opened in the debugger regardless of the outcome of the test.
    ///
    /// If the matching test is a fuzz test, then it will open the debugger on the first failure
    /// case.
    /// If the fuzz test does not fail, it will open the debugger on the last fuzz case.
    ///
    /// For more fine-grained control of which fuzz case is run, see forge run.
    #[clap(long, value_name = "TEST_FUNCTION")]
    debug: Option<Regex>,

    /// Print a gas report.
    #[clap(long, env = "FORGE_GAS_REPORT")]
    gas_report: bool,

    /// Exit with code 0 even if a test fails.
    #[clap(long, env = "FORGE_ALLOW_FAILURE")]
    allow_failure: bool,

    /// Output test results in JSON format.
    #[clap(long, short, help_heading = "Display options")]
    json: bool,

    #[clap(flatten)]
    evm_opts: EvmArgs,

    /// The Etherscan (or equivalent) API key
    #[clap(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    etherscan_api_key: Option<String>,

    #[clap(flatten)]
    opts: CoreBuildArgs,

    #[clap(flatten)]
    pub watch: WatchArgs,

    /// List tests instead of running them
    #[clap(long, short, help_heading = "Display options")]
    list: bool,

    /// Set seed used to generate randomness during your fuzz runs.
    #[clap(long, value_parser = utils::parse_u256)]
    pub fuzz_seed: Option<U256>,
}

impl TestArgs {
    /// Returns the flattened [`CoreBuildArgs`].
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }

    /// Executes all the tests in the project.
    ///
    /// This will trigger the build process first. On success all test contracts that match the
    /// configured filter will be executed
    ///
    /// Returns the test results for all matching tests.
    pub fn execute_tests(self) -> eyre::Result<TestOutcome> {
        // Merge all configs
        let (mut config, mut evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;

        let test_options = TestOptions { fuzz: config.fuzz, invariant: config.invariant };

        let mut filter = self.filter(&config);

        trace!(target: "forge::test", ?filter, "using filter");

        // Set up the project
        let mut project = config.project()?;

        // install missing dependencies
        if install::install_missing_dependencies(&mut config, &project, self.build_args().silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
            project = config.project()?;
        }

        let compiler = ProjectCompiler::default();
        let output = if config.sparse_mode {
            compiler.compile_sparse(&project, filter.clone())
        } else if self.opts.silent {
            compile::suppress_compile(&project)
        } else {
            compiler.compile(&project)
        }?;

        // Determine print verbosity and executor verbosity
        let verbosity = evm_opts.verbosity;
        if self.gas_report && evm_opts.verbosity < 3 {
            evm_opts.verbosity = 3;
        }

        let env = evm_opts.evm_env_blocking()?;

        // Prepare the test builder
        let evm_spec = utils::evm_spec(&config.evm_version);

        let mut runner = MultiContractRunnerBuilder::default()
            .initial_balance(evm_opts.initial_balance)
            .evm_spec(evm_spec)
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, env.clone()))
            .with_cheats_config(CheatsConfig::new(&config, &evm_opts))
            .with_test_options(test_options)
            .build(project.paths.root, output, env, evm_opts)?;

        if self.debug.is_some() {
            filter.args_mut().test_pattern = self.debug;

            match runner.count_filtered_tests(&filter) {
                1 => {
                    // Run the test
                    let results = runner.test(&filter, None, test_options)?;

                    // Get the result of the single test
                    let (id, sig, test_kind, counterexample, breakpoints) = results.iter().map(|(id, SuiteResult{ test_results, .. })| {
                        let (sig, result) = test_results.iter().next().unwrap();

                        (id.clone(), sig.clone(), result.kind.clone(), result.counterexample.clone(), result.breakpoints.clone())
                    }).next().unwrap();

                    // Build debugger args if this is a fuzz test
                    let sig = match test_kind {
                        TestKind::Fuzz { first_case, .. } => {
                            if let Some(CounterExample::Single(counterexample)) = counterexample {
                                counterexample.calldata.to_string()
                            } else {
                                first_case.calldata.to_string()
                            }
                        },
                        _ => sig,
                    };

                    // Run the debugger
                    let mut opts = self.opts.clone();
                    opts.silent = true;
                    let debugger = DebugArgs {
                        path: PathBuf::from(runner.source_paths.get(&id).unwrap()),
                        target_contract: Some(get_contract_name(&id).to_string()),
                        sig,
                        args: Vec::new(),
                        debug: true,
                        opts,
                        evm_opts: self.evm_opts,
                    };
                    utils::block_on(debugger.debug(breakpoints))?;

                    Ok(TestOutcome::new(results, self.allow_failure))
                }
                n =>
                    Err(
                        eyre::eyre!("{n} tests matched your criteria, but exactly 1 test must match in order to run the debugger.\n
                        \n
                        Use --match-contract and --match-path to further limit the search."))
            }
        } else if self.list {
            list(runner, filter, self.json)
        } else {
            test(
                config,
                runner,
                verbosity,
                filter,
                self.json,
                self.allow_failure,
                test_options,
                self.gas_report,
            )
        }
    }

    /// Returns the flattened [`FilterArgs`] arguments merged with [`Config`].
    pub fn filter(&self, config: &Config) -> ProjectPathsAwareFilter {
        self.filter.merge_with_config(config)
    }

    /// Returns whether `BuildArgs` was configured with `--watch`
    pub fn is_watch(&self) -> bool {
        self.watch.watch.is_some()
    }

    /// Returns the [`watchexec::InitConfig`] and [`watchexec::RuntimeConfig`] necessary to
    /// bootstrap a new [`watchexe::Watchexec`] loop.
    pub(crate) fn watchexec_config(&self) -> eyre::Result<(InitConfig, RuntimeConfig)> {
        self.watch.watchexec_config(|| {
            let config = Config::from(self);
            vec![config.src, config.test]
        })
    }
}

impl Provider for TestArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Core Build Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();

        let mut fuzz_dict = Dict::default();
        if let Some(fuzz_seed) = self.fuzz_seed {
            fuzz_dict.insert("seed".to_string(), fuzz_seed.to_string().into());
        }
        dict.insert("fuzz".to_string(), fuzz_dict.into());

        if let Some(ref etherscan_api_key) = self.etherscan_api_key {
            dict.insert("etherscan_api_key".to_string(), etherscan_api_key.to_string().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

impl Cmd for TestArgs {
    type Output = TestOutcome;

    fn run(self) -> eyre::Result<Self::Output> {
        trace!(target: "forge::test", "executing test command");
        shell::set_shell(shell::Shell::from_args(self.opts.silent, self.json))?;
        self.execute_tests()
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
    pub result: TestResult,
}

impl Test {
    pub fn gas_used(&self) -> u64 {
        self.result.kind.report().gas()
    }

    /// Returns the contract name of the artifact id
    pub fn contract_name(&self) -> &str {
        get_contract_name(&self.artifact_id)
    }

    /// Returns the file name of the artifact id
    pub fn file_name(&self) -> &str {
        get_file_name(&self.artifact_id)
    }
}

/// Represents the bundled results of all tests
pub struct TestOutcome {
    /// Whether failures are allowed
    pub allow_failure: bool,
    /// Results for each suite of tests `contract -> SuiteResult`
    pub results: BTreeMap<String, SuiteResult>,
}

impl TestOutcome {
    fn new(results: BTreeMap<String, SuiteResult>, allow_failure: bool) -> Self {
        Self { results, allow_failure }
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
        self.results.values().flat_map(|suite| suite.tests())
    }

    /// Returns an iterator over all `Test`
    pub fn into_tests(self) -> impl Iterator<Item = Test> {
        self.results
            .into_iter()
            .flat_map(|(file, SuiteResult { test_results, .. })| {
                test_results.into_iter().map(move |t| (file.clone(), t))
            })
            .map(|(artifact_id, (signature, result))| Test { artifact_id, signature, result })
    }

    /// Checks if there are any failures and failures are disallowed
    pub fn ensure_ok(&self) -> eyre::Result<()> {
        let failures = self.failures().count();
        if self.allow_failure || failures == 0 {
            return Ok(())
        }

        if !shell::verbosity().is_normal() {
            // skip printing and exit early
            std::process::exit(1);
        }

        println!();
        println!("Failing tests:");
        for (suite_name, suite) in self.results.iter() {
            let failures = suite.failures().count();
            if failures == 0 {
                continue
            }

            let term = if failures > 1 { "tests" } else { "test" };
            println!("Encountered {failures} failing {term} in {suite_name}");
            for (name, result) in suite.failures() {
                short_test_result(name, result);
            }
            println!();
        }

        let successes = self.successes().count();
        println!(
            "Encountered a total of {} failing tests, {} tests succeeded",
            Paint::red(failures.to_string()),
            Paint::green(successes.to_string())
        );
        std::process::exit(1);
    }

    pub fn duration(&self) -> Duration {
        self.results
            .values()
            .fold(Duration::ZERO, |acc, SuiteResult { duration, .. }| acc + *duration)
    }

    pub fn summary(&self) -> String {
        let failed = self.failures().count();
        let result = if failed == 0 { Paint::green("ok") } else { Paint::red("FAILED") };
        format!(
            "Test result: {}. {} passed; {} failed; finished in {:.2?}",
            result,
            self.successes().count(),
            failed,
            self.duration()
        )
    }
}

fn short_test_result(name: &str, result: &TestResult) {
    let status = if result.success {
        Paint::green("[PASS]".to_string())
    } else {
        let reason = result
            .reason
            .as_ref()
            .map(|reason| format!("Reason: {reason}"))
            .unwrap_or_else(|| "Reason: Assertion failed.".to_string());

        let counterexample = result
            .counterexample
            .as_ref()
            .map(|example| match example {
                CounterExample::Single(eg) => format!(" Counterexample: {eg}]"),
                CounterExample::Sequence(sequence) => {
                    let mut inner_txt = String::new();

                    for checkpoint in sequence {
                        inner_txt += format!("\t\t{checkpoint}\n").as_str();
                    }
                    format!("]\n\t[Sequence]\n{inner_txt}\n")
                }
            })
            .unwrap_or_else(|| "]".to_string());

        Paint::red(format!("[FAIL. {reason}{counterexample}"))
    };

    println!("{status} {name} {}", result.kind.report());
}

/// Lists all matching tests
fn list(
    runner: MultiContractRunner,
    filter: ProjectPathsAwareFilter,
    json: bool,
) -> eyre::Result<TestOutcome> {
    let results = runner.list(&filter);

    if json {
        println!("{}", serde_json::to_string(&results)?);
    } else {
        for (file, contracts) in results.iter() {
            println!("{file}");
            for (contract, tests) in contracts.iter() {
                println!("  {contract}");
                println!("    {}\n", tests.join("\n    "));
            }
        }
    }
    Ok(TestOutcome::new(BTreeMap::new(), false))
}

/// Runs all the tests
#[allow(clippy::too_many_arguments)]
fn test(
    config: Config,
    mut runner: MultiContractRunner,
    verbosity: u8,
    filter: ProjectPathsAwareFilter,
    json: bool,
    allow_failure: bool,
    test_options: TestOptions,
    gas_reporting: bool,
) -> eyre::Result<TestOutcome> {
    trace!(target: "forge::test", "running all tests");
    if runner.count_filtered_tests(&filter) == 0 {
        let filter_str = filter.to_string();
        if filter_str.is_empty() {
            println!(
                "\nNo tests found in project! Forge looks for functions that starts with `test`."
            );
        } else {
            println!("\nNo tests match the provided pattern:");
            println!("{filter_str}");
            // Try to suggest a test when there's no match
            if let Some(ref test_pattern) = filter.args().test_pattern {
                let test_name = test_pattern.as_str();
                let candidates = runner.get_tests(&filter);
                if let Some(suggestion) = suggestions::did_you_mean(test_name, candidates).pop() {
                    println!("\nDid you mean `{suggestion}`?");
                }
            }
        }
    }

    if json {
        let results = runner.test(&filter, None, test_options)?;
        println!("{}", serde_json::to_string(&results)?);
        Ok(TestOutcome::new(results, allow_failure))
    } else {
        // Set up identifiers
        let mut local_identifier = LocalTraceIdentifier::new(&runner.known_contracts);
        let remote_chain_id = runner.evm_opts.get_remote_chain_id();
        // Do not re-query etherscan for contracts that you've already queried today.
        let mut etherscan_identifier = EtherscanIdentifier::new(&config, remote_chain_id)?;

        // Set up test reporter channel
        let (tx, rx) = channel::<(String, SuiteResult)>();

        // Run tests
        let handle = thread::spawn(move || runner.test(&filter, Some(tx), test_options).unwrap());

        let mut results: BTreeMap<String, SuiteResult> = BTreeMap::new();
        let mut gas_report = GasReport::new(config.gas_reports, config.gas_reports_ignore);
        let sig_identifier =
            SignaturesIdentifier::new(Config::foundry_cache_dir(), config.offline)?;

        for (contract_name, suite_result) in rx {
            let mut tests = suite_result.test_results.clone();
            println!();
            for warning in suite_result.warnings.iter() {
                eprintln!("{} {warning}", Paint::yellow("Warning:").bold());
            }
            if !tests.is_empty() {
                let term = if tests.len() > 1 { "tests" } else { "test" };
                println!("Running {} {term} for {contract_name}", tests.len());
            }
            for (name, result) in &mut tests {
                short_test_result(name, result);

                // We only display logs at level 2 and above
                if verbosity >= 2 {
                    // We only decode logs from Hardhat and DS-style console events
                    let console_logs = decode_console_logs(&result.logs);
                    if !console_logs.is_empty() {
                        println!("Logs:");
                        for log in console_logs {
                            println!("  {log}");
                        }
                        println!();
                    }
                }

                if !result.traces.is_empty() {
                    // Identify addresses in each trace
                    let mut decoder = CallTraceDecoderBuilder::new()
                        .with_labels(result.labeled_addresses.clone())
                        .with_events(local_identifier.events())
                        .with_verbosity(verbosity)
                        .build();

                    // Signatures are of no value for gas reports
                    if !gas_reporting {
                        decoder.add_signature_identifier(sig_identifier.clone());
                    }

                    // Decode the traces
                    let mut decoded_traces = Vec::new();
                    let rt = RuntimeOrHandle::new();
                    for (kind, trace) in &mut result.traces {
                        decoder.identify(trace, &mut local_identifier);
                        decoder.identify(trace, &mut etherscan_identifier);

                        let should_include = match kind {
                            // At verbosity level 3, we only display traces for failed tests
                            // At verbosity level 4, we also display the setup trace for failed
                            // tests At verbosity level 5, we display
                            // all traces for all tests
                            TraceKind::Setup => {
                                (verbosity >= 5) || (verbosity == 4 && !result.success)
                            }
                            TraceKind::Execution => {
                                verbosity > 3 || (verbosity == 3 && !result.success)
                            }
                            _ => false,
                        };

                        // We decode the trace if we either need to build a gas report or we need
                        // to print it
                        if should_include || gas_reporting {
                            rt.block_on(decoder.decode(trace));
                        }

                        if should_include {
                            decoded_traces.push(trace.to_string());
                        }
                    }

                    if !decoded_traces.is_empty() {
                        println!("Traces:");
                        decoded_traces.into_iter().for_each(|trace| println!("{trace}"));
                    }

                    if gas_reporting {
                        gas_report.analyze(&result.traces);
                    }
                }
            }
            let block_outcome = TestOutcome::new(
                [(contract_name.clone(), suite_result.clone())].into(),
                allow_failure,
            );
            println!("{}", block_outcome.summary());
            results.insert(contract_name, suite_result);
        }

        if gas_reporting {
            println!("{}", gas_report.finalize());
        }

        // reattach the thread
        let _ = handle.join();

        trace!(target: "forge::test", "received {} results", results.len());
        Ok(TestOutcome::new(results, allow_failure))
    }
}
