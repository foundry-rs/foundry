use super::{install, test::filter::ProjectPathsAwareFilter, watch::WatchArgs};
use alloy_primitives::U256;
use clap::Parser;
use eyre::Result;
use forge::{
    decode::decode_console_logs,
    gas_report::GasReport,
    inspectors::CheatsConfig,
    result::{SuiteResult, TestResult, TestStatus},
    traces::{
        identifier::{EtherscanIdentifier, LocalTraceIdentifier, SignaturesIdentifier},
        CallTraceDecoderBuilder, TraceKind,
    },
    MultiContractRunner, MultiContractRunnerBuilder, TestOptions, TestOptionsBuilder,
};
use foundry_cli::{
    opts::CoreBuildArgs,
    utils::{self, LoadConfig},
};
use foundry_common::{
    compact_to_contract,
    compile::{self, ContractSources, ProjectCompiler},
    evm::EvmArgs,
    get_contract_name, get_file_name, shell,
};
use foundry_config::{
    figment,
    figment::{
        value::{Dict, Map},
        Metadata, Profile, Provider,
    },
    get_available_profiles, Config,
};
use foundry_debugger::DebuggerArgs;
use foundry_evm::fuzz::CounterExample;
use regex::Regex;
use std::{collections::BTreeMap, fs, sync::mpsc::channel, time::Duration};
use tracing::trace;
use watchexec::config::{InitConfig, RuntimeConfig};
use yansi::Paint;

mod filter;
mod summary;
use summary::TestSummaryReporter;

pub use filter::FilterArgs;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(TestArgs, opts, evm_opts);

/// CLI arguments for `forge test`.
#[derive(Debug, Clone, Parser)]
#[clap(next_help_heading = "Test options")]
pub struct TestArgs {
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

    /// Stop running tests after the first failure
    #[clap(long)]
    pub fail_fast: bool,

    /// The Etherscan (or equivalent) API key
    #[clap(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    etherscan_api_key: Option<String>,

    /// List tests instead of running them
    #[clap(long, short, help_heading = "Display options")]
    list: bool,

    /// Set seed used to generate randomness during your fuzz runs.
    #[clap(long)]
    pub fuzz_seed: Option<U256>,

    #[clap(long, env = "FOUNDRY_FUZZ_RUNS", value_name = "RUNS")]
    pub fuzz_runs: Option<u64>,

    #[clap(flatten)]
    filter: FilterArgs,

    #[clap(flatten)]
    evm_opts: EvmArgs,

    #[clap(flatten)]
    opts: CoreBuildArgs,

    #[clap(flatten)]
    pub watch: WatchArgs,

    /// Print test summary table
    #[clap(long, help_heading = "Display options")]
    pub summary: bool,

    /// Print detailed test summary table
    #[clap(long, help_heading = "Display options")]
    pub detailed: bool,
}

impl TestArgs {
    /// Returns the flattened [`CoreBuildArgs`].
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }

    pub async fn run(self) -> Result<TestOutcome> {
        trace!(target: "forge::test", "executing test command");
        shell::set_shell(shell::Shell::from_args(self.opts.silent, self.json))?;
        self.execute_tests().await
    }

    /// Executes all the tests in the project.
    ///
    /// This will trigger the build process first. On success all test contracts that match the
    /// configured filter will be executed
    ///
    /// Returns the test results for all matching tests.
    pub async fn execute_tests(self) -> Result<TestOutcome> {
        // Merge all configs
        let (mut config, mut evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;

        let mut filter = self.filter(&config);

        trace!(target: "forge::test", ?filter, "using filter");

        // Set up the project
        let mut project = config.project()?;

        // install missing dependencies
        if install::install_missing_dependencies(&mut config, self.build_args().silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
            project = config.project()?;
        }

        let compiler = ProjectCompiler::default();
        let output = match (config.sparse_mode, self.opts.silent | self.json) {
            (false, false) => compiler.compile(&project),
            (true, false) => compiler.compile_sparse(&project, filter.clone()),
            (false, true) => compile::suppress_compile(&project),
            (true, true) => compile::suppress_compile_sparse(&project, filter.clone()),
        }?;
        // Create test options from general project settings
        // and compiler output
        let project_root = &project.paths.root;
        let toml = config.get_config_path();
        let profiles = get_available_profiles(toml)?;

        let test_options: TestOptions = TestOptionsBuilder::default()
            .fuzz(config.fuzz)
            .invariant(config.invariant)
            .profiles(profiles)
            .build(&output, project_root)?;

        // Determine print verbosity and executor verbosity
        let verbosity = evm_opts.verbosity;
        if self.gas_report && evm_opts.verbosity < 3 {
            evm_opts.verbosity = 3;
        }

        let env = evm_opts.evm_env().await?;

        // Prepare the test builder
        let should_debug = self.debug.is_some();

        let mut runner_builder = MultiContractRunnerBuilder::default()
            .set_debug(should_debug)
            .initial_balance(evm_opts.initial_balance)
            .evm_spec(config.evm_spec_id())
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, env.clone()))
            .with_cheats_config(CheatsConfig::new(&config, evm_opts.clone()))
            .with_test_options(test_options.clone());

        let mut runner = runner_builder.clone().build(
            project_root,
            output.clone(),
            env.clone(),
            evm_opts.clone(),
        )?;

        if should_debug {
            filter.args_mut().test_pattern = self.debug.clone();
            let num_filtered = runner.matching_test_function_count(&filter);
            if num_filtered != 1 {
                return Err(
                        eyre::eyre!("{num_filtered} tests matched your criteria, but exactly 1 test must match in order to run the debugger.\n
                        \n
                        Use --match-contract and --match-path to further limit the search."));
            }
            let test_funcs = runner.get_matching_test_functions(&filter);
            // if we debug a fuzz test, we should not collect data on the first run
            if !test_funcs.first().expect("matching function exists").inputs.is_empty() {
                runner_builder = runner_builder.set_debug(false);
                runner = runner_builder.clone().build(
                    project_root,
                    output.clone(),
                    env.clone(),
                    evm_opts.clone(),
                )?;
            }
        }

        let known_contracts = runner.known_contracts.clone();
        let mut local_identifier = LocalTraceIdentifier::new(&known_contracts);
        let remote_chain_id = runner.evm_opts.get_remote_chain_id();

        let outcome = self
            .run_tests(runner, config.clone(), verbosity, filter.clone(), test_options.clone())
            .await?;

        if should_debug {
            let tests = outcome.clone().into_tests();
            let mut decoders = Vec::new();
            for test in tests {
                let mut result = test.result;
                // Identify addresses in each trace
                let mut builder = CallTraceDecoderBuilder::new()
                    .with_labels(result.labeled_addresses.clone())
                    .with_events(local_identifier.events().cloned())
                    .with_verbosity(verbosity);

                // Signatures are of no value for gas reports
                if !self.gas_report {
                    let sig_identifier =
                        SignaturesIdentifier::new(Config::foundry_cache_dir(), config.offline)?;
                    builder = builder.with_signature_identifier(sig_identifier.clone());
                }

                let mut decoder = builder.build();

                if !result.traces.is_empty() {
                    // Set up identifiers
                    // Do not re-query etherscan for contracts that you've already queried today.
                    let mut etherscan_identifier =
                        EtherscanIdentifier::new(&config, remote_chain_id)?;

                    // Decode the traces
                    for (kind, trace) in &mut result.traces {
                        decoder.identify(trace, &mut local_identifier);
                        decoder.identify(trace, &mut etherscan_identifier);

                        let should_include = match kind {
                            // At verbosity level 3, we only display traces for failed tests
                            // At verbosity level 4, we also display the setup trace for failed
                            // tests At verbosity level 5, we display
                            // all traces for all tests
                            TraceKind::Setup => {
                                (verbosity >= 5) ||
                                    (verbosity == 4 && result.status == TestStatus::Failure)
                            }
                            TraceKind::Execution => {
                                verbosity > 3 ||
                                    (verbosity == 3 && result.status == TestStatus::Failure)
                            }
                            _ => false,
                        };

                        // We decode the trace if we either need to build a gas report or we need
                        // to print it
                        if should_include || self.gas_report {
                            decoder.decode(trace).await;
                        }
                    }
                }

                decoders.push(decoder);
            }

            let mut sources: ContractSources = Default::default();
            for (id, artifact) in output.into_artifacts() {
                // Sources are only required for the debugger, but it *might* mean that there's
                // something wrong with the build and/or artifacts.
                if let Some(source) = artifact.source_file() {
                    let abs_path = source
                        .ast
                        .ok_or(eyre::eyre!("Source from artifact has no AST."))?
                        .absolute_path;
                    let source_code = fs::read_to_string(abs_path)?;
                    let contract = artifact.clone().into_contract_bytecode();
                    let source_contract = compact_to_contract(contract)?;
                    sources
                        .0
                        .entry(id.clone().name)
                        .or_default()
                        .insert(source.id, (source_code, source_contract));
                }
            }

            let test = outcome.clone().into_tests().next().unwrap();
            let result = test.result;
            // Run the debugger
            let debugger = DebuggerArgs {
                debug: result.debug.map_or(vec![], |debug| vec![debug]),
                decoder: decoders.first().unwrap(),
                sources,
                breakpoints: result.breakpoints,
            };
            debugger.run()?;
        }

        Ok(outcome)
    }

    /// Run all tests that matches the filter predicate from a test runner
    pub async fn run_tests(
        &self,
        mut runner: MultiContractRunner,
        config: Config,
        verbosity: u8,
        mut filter: ProjectPathsAwareFilter,
        test_options: TestOptions,
    ) -> eyre::Result<TestOutcome> {
        if self.debug.is_some() {
            filter.args_mut().test_pattern = self.debug.clone();
            // Run the test
            let results = runner.test(&filter, None, test_options).await;

            Ok(TestOutcome::new(results, self.allow_failure))
        } else if self.list {
            list(runner, filter, self.json)
        } else {
            if self.detailed && !self.summary {
                return Err(eyre::eyre!(
                    "Missing `--summary` option in your command. You must pass it along with the `--detailed` option to view detailed test summary."
                ));
            }
            test(
                config,
                runner,
                verbosity,
                filter,
                self.json,
                self.allow_failure,
                test_options,
                self.gas_report,
                self.fail_fast,
                self.summary,
                self.detailed,
            )
            .await
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
    pub(crate) fn watchexec_config(&self) -> Result<(InitConfig, RuntimeConfig)> {
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
        if let Some(fuzz_runs) = self.fuzz_runs {
            fuzz_dict.insert("runs".to_string(), fuzz_runs.into());
        }
        dict.insert("fuzz".to_string(), fuzz_dict.into());

        if let Some(ref etherscan_api_key) = self.etherscan_api_key {
            dict.insert("etherscan_api_key".to_string(), etherscan_api_key.to_string().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
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
#[derive(Clone)]
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
        self.tests().filter(|(_, t)| t.status == TestStatus::Success)
    }

    /// Iterator over all failing tests and their names
    pub fn failures(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Failure)
    }

    pub fn skips(&self) -> impl Iterator<Item = (&String, &TestResult)> {
        self.tests().filter(|(_, t)| t.status == TestStatus::Skipped)
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
    pub fn ensure_ok(&self) -> Result<()> {
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
            "Test result: {}. {} passed; {} failed; {} skipped; finished in {:.2?}",
            result,
            Paint::green(self.successes().count()),
            Paint::red(failed),
            Paint::yellow(self.skips().count()),
            self.duration()
        )
    }
}

fn short_test_result(name: &str, result: &TestResult) {
    let status = if result.status == TestStatus::Success {
        Paint::green("[PASS]".to_string())
    } else if result.status == TestStatus::Skipped {
        Paint::yellow("[SKIP]".to_string())
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

/**
 * Formats the aggregated summary of all test suites into a string (for printing)
 */
fn format_aggregated_summary(
    num_test_suites: usize,
    total_passed: usize,
    total_failed: usize,
    total_skipped: usize,
) -> String {
    let total_tests = total_passed + total_failed + total_skipped;
    format!(
        " \nRan {} test suites: {} tests passed, {} failed, {} skipped ({} total tests)",
        num_test_suites,
        Paint::green(total_passed),
        Paint::red(total_failed),
        Paint::yellow(total_skipped),
        total_tests
    )
}

/// Lists all matching tests
fn list(
    runner: MultiContractRunner,
    filter: ProjectPathsAwareFilter,
    json: bool,
) -> Result<TestOutcome> {
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
async fn test(
    config: Config,
    mut runner: MultiContractRunner,
    verbosity: u8,
    filter: ProjectPathsAwareFilter,
    json: bool,
    allow_failure: bool,
    test_options: TestOptions,
    gas_reporting: bool,
    fail_fast: bool,
    summary: bool,
    detailed: bool,
) -> Result<TestOutcome> {
    trace!(target: "forge::test", "running all tests");

    if runner.matching_test_function_count(&filter) == 0 {
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
                if let Some(suggestion) = utils::did_you_mean(test_name, candidates).pop() {
                    println!("\nDid you mean `{suggestion}`?");
                }
            }
        }
    }

    if json {
        let results = runner.test(filter, None, test_options).await;
        println!("{}", serde_json::to_string(&results)?);
        return Ok(TestOutcome::new(results, allow_failure))
    }

    // Set up identifiers
    let known_contracts = runner.known_contracts.clone();
    let mut local_identifier = LocalTraceIdentifier::new(&known_contracts);
    let remote_chain_id = runner.evm_opts.get_remote_chain_id();
    // Do not re-query etherscan for contracts that you've already queried today.
    let mut etherscan_identifier = EtherscanIdentifier::new(&config, remote_chain_id)?;

    // Set up test reporter channel
    let (tx, rx) = channel::<(String, SuiteResult)>();

    // Run tests
    let handle =
        tokio::task::spawn(async move { runner.test(filter, Some(tx), test_options).await });

    let mut results = BTreeMap::new();
    let mut gas_report = GasReport::new(config.gas_reports, config.gas_reports_ignore);
    let sig_identifier = SignaturesIdentifier::new(Config::foundry_cache_dir(), config.offline)?;

    let mut total_passed = 0;
    let mut total_failed = 0;
    let mut total_skipped = 0;
    let mut suite_results: Vec<TestOutcome> = Vec::new();

    'outer: for (contract_name, suite_result) in rx {
        results.insert(contract_name.clone(), suite_result.clone());

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

            // If the test failed, we want to stop processing the rest of the tests
            if fail_fast && result.status == TestStatus::Failure {
                break 'outer
            }

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

            if result.traces.is_empty() {
                continue
            }

            // Identify addresses in each trace
            let mut builder = CallTraceDecoderBuilder::new()
                .with_labels(result.labeled_addresses.iter().map(|(a, s)| (*a, s.clone())))
                .with_events(local_identifier.events().cloned())
                .with_verbosity(verbosity);

            // Signatures are of no value for gas reports
            if !gas_reporting {
                builder = builder.with_signature_identifier(sig_identifier.clone());
            }

            let mut decoder = builder.build();

            // Decode the traces
            let mut decoded_traces = Vec::with_capacity(result.traces.len());
            for (kind, trace) in &mut result.traces {
                decoder.identify(trace, &mut local_identifier);
                decoder.identify(trace, &mut etherscan_identifier);

                // verbosity:
                // - 0..3: nothing
                // - 3: only display traces for failed tests
                // - 4: also display the setup trace for failed tests
                // - 5..: display all traces for all tests
                let should_include = match kind {
                    TraceKind::Execution => {
                        (verbosity == 3 && result.status.is_failure()) || verbosity >= 4
                    }
                    TraceKind::Setup => {
                        (verbosity == 4 && result.status.is_failure()) || verbosity >= 5
                    }
                    TraceKind::Deployment => false,
                };

                // Decode the trace if we either need to build a gas report or we need to print it
                if should_include || gas_reporting {
                    decoder.decode(trace).await;
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
        let block_outcome =
            TestOutcome::new([(contract_name.clone(), suite_result)].into(), allow_failure);

        total_passed += block_outcome.successes().count();
        total_failed += block_outcome.failures().count();
        total_skipped += block_outcome.skips().count();

        println!("{}", block_outcome.summary());

        if summary {
            suite_results.push(block_outcome.clone());
        }
    }

    if gas_reporting {
        println!("{}", gas_report.finalize());
    }

    let num_test_suites = results.len();

    if num_test_suites > 0 {
        println!(
            "{}",
            format_aggregated_summary(num_test_suites, total_passed, total_failed, total_skipped)
        );

        if summary {
            let mut summary_table = TestSummaryReporter::new(detailed);
            println!("\n\nTest Summary:");
            summary_table.print_summary(suite_results);
        }
    }

    // reattach the task
    let _results = handle.await?;

    trace!(target: "forge::test", "received {} results", results.len());

    Ok(TestOutcome::new(results, allow_failure))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sanity test that unknown args are rejected
    #[test]
    fn test_verbosity() {
        #[derive(Debug, Parser)]
        pub struct VerbosityArgs {
            #[clap(long, short, action = clap::ArgAction::Count)]
            verbosity: u8,
        }
        let res = VerbosityArgs::try_parse_from(["foundry-cli", "-vw"]);
        assert!(res.is_err());

        let res = VerbosityArgs::try_parse_from(["foundry-cli", "-vv"]);
        assert!(res.is_ok());
    }

    #[test]
    fn test_verbosity_multi_short() {
        #[derive(Debug, Parser)]
        pub struct VerbosityArgs {
            #[clap(long, short)]
            verbosity: bool,
            #[clap(
                long,
                short,
                num_args(0..),
                value_name = "PATH",
            )]
            watch: Option<Vec<String>>,
        }
        // this is supported by clap
        let res = VerbosityArgs::try_parse_from(["foundry-cli", "-vw"]);
        assert!(res.is_ok())
    }

    #[test]
    fn test_watch_parse() {
        let args: TestArgs = TestArgs::parse_from(["foundry-cli", "-vw"]);
        assert!(args.watch.watch.is_some());
    }

    #[test]
    fn test_fuzz_seed() {
        let args: TestArgs = TestArgs::parse_from(["foundry-cli", "--fuzz-seed", "0x10"]);
        assert!(args.fuzz_seed.is_some());
    }

    // <https://github.com/foundry-rs/foundry/issues/5913>
    #[test]
    fn test_5913() {
        let args: TestArgs =
            TestArgs::parse_from(["foundry-cli", "-vvv", "--gas-report", "--fuzz-seed", "0x10"]);
        assert!(args.fuzz_seed.is_some());
    }
}
