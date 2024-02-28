use super::{install, test::filter::ProjectPathsAwareFilter, watch::WatchArgs};
use alloy_primitives::U256;
use clap::Parser;
use eyre::Result;
use forge::{
    decode::decode_console_logs,
    gas_report::GasReport,
    inspectors::CheatsConfig,
    result::{SuiteResult, TestOutcome, TestStatus},
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
    compile::{ContractSources, ProjectCompiler},
    evm::EvmArgs,
    shell,
};
use foundry_config::{
    figment,
    figment::{
        value::{Dict, Map},
        Metadata, Profile, Provider,
    },
    get_available_profiles, Config,
};
use foundry_debugger::Debugger;
use regex::Regex;
use std::{sync::mpsc::channel, time::Instant};
use watchexec::config::{InitConfig, RuntimeConfig};
use yansi::Paint;

mod filter;
mod summary;
use summary::TestSummaryReporter;

pub use filter::FilterArgs;
use forge::traces::render_trace_arena;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(TestArgs, opts, evm_opts);

/// CLI arguments for `forge test`.
#[derive(Clone, Debug, Parser)]
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

    /// Stop running tests after the first failure.
    #[clap(long)]
    pub fail_fast: bool,

    /// The Etherscan (or equivalent) API key.
    #[clap(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    etherscan_api_key: Option<String>,

    /// List tests instead of running them.
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

    /// Print test summary table.
    #[clap(long, help_heading = "Display options")]
    pub summary: bool,

    /// Print detailed test summary table.
    #[clap(long, help_heading = "Display options", requires = "summary")]
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

        // Explicitly enable isolation for gas reports for more correct gas accounting
        if self.gas_report {
            evm_opts.isolate = true;
        }

        // Set up the project.
        let mut project = config.project()?;

        // Install missing dependencies.
        if install::install_missing_dependencies(&mut config, self.build_args().silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
            project = config.project()?;
        }

        let mut filter = self.filter(&config);
        trace!(target: "forge::test", ?filter, "using filter");

        let mut compiler = ProjectCompiler::new().quiet_if(self.json || self.opts.silent);
        if config.sparse_mode {
            compiler = compiler.filter(Box::new(filter.clone()));
        }
        let output = compiler.compile(&project)?;

        // Create test options from general project settings and compiler output.
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

        // Clone the output only if we actually need it later for the debugger.
        let output_clone = should_debug.then(|| output.clone());

        let runner = MultiContractRunnerBuilder::default()
            .set_debug(should_debug)
            .initial_balance(evm_opts.initial_balance)
            .evm_spec(config.evm_spec_id())
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, env.clone()))
            .with_cheats_config(CheatsConfig::new(&config, evm_opts.clone(), None))
            .with_test_options(test_options.clone())
            .enable_isolation(evm_opts.isolate)
            .build(project_root, output, env, evm_opts)?;

        if let Some(debug_test_pattern) = &self.debug {
            let test_pattern = &mut filter.args_mut().test_pattern;
            if test_pattern.is_some() {
                eyre::bail!(
                    "Cannot specify both --debug and --match-test. \
                     Use --match-contract and --match-path to further limit the search instead."
                );
            }
            *test_pattern = Some(debug_test_pattern.clone());
        }

        let outcome = self.run_tests(runner, config, verbosity, &filter, test_options).await?;

        if should_debug {
            // There is only one test.
            let Some(test) = outcome.into_tests_cloned().next() else {
                return Err(eyre::eyre!("no tests were executed"));
            };

            let sources = ContractSources::from_project_output(
                output_clone.as_ref().unwrap(),
                project.root(),
            )?;

            // Run the debugger.
            let mut builder = Debugger::builder()
                .debug_arenas(test.result.debug.as_slice())
                .sources(sources)
                .breakpoints(test.result.breakpoints);
            if let Some(decoder) = &outcome.decoder {
                builder = builder.decoder(decoder);
            }
            let mut debugger = builder.build();
            debugger.try_run()?;
        }

        Ok(outcome)
    }

    /// Run all tests that matches the filter predicate from a test runner
    pub async fn run_tests(
        &self,
        mut runner: MultiContractRunner,
        config: Config,
        verbosity: u8,
        filter: &ProjectPathsAwareFilter,
        test_options: TestOptions,
    ) -> eyre::Result<TestOutcome> {
        if self.list {
            return list(runner, filter, self.json);
        }

        trace!(target: "forge::test", "running all tests");

        let num_filtered = runner.matching_test_function_count(filter);
        if num_filtered == 0 {
            println!();
            if filter.is_empty() {
                println!(
                    "No tests found in project! \
                     Forge looks for functions that starts with `test`."
                );
            } else {
                println!("No tests match the provided pattern:");
                print!("{filter}");

                // Try to suggest a test when there's no match
                if let Some(test_pattern) = &filter.args().test_pattern {
                    let test_name = test_pattern.as_str();
                    let candidates = runner.get_tests(filter);
                    if let Some(suggestion) = utils::did_you_mean(test_name, candidates).pop() {
                        println!("\nDid you mean `{suggestion}`?");
                    }
                }
            }
        }
        if self.debug.is_some() && num_filtered != 1 {
            eyre::bail!(
                "{num_filtered} tests matched your criteria, but exactly 1 test must match in order to run the debugger.\n\n\
                 Use --match-contract and --match-path to further limit the search.\n\
                 Filter used:\n{filter}"
            );
        }

        if self.json {
            let results = runner.test_collect(filter, test_options).await;
            println!("{}", serde_json::to_string(&results)?);
            return Ok(TestOutcome::new(results, self.allow_failure));
        }

        // Set up trace identifiers.
        let known_contracts = runner.known_contracts.clone();
        let mut local_identifier = LocalTraceIdentifier::new(&known_contracts);
        let remote_chain_id = runner.evm_opts.get_remote_chain_id();
        let mut etherscan_identifier = EtherscanIdentifier::new(&config, remote_chain_id)?;

        // Run tests.
        let (tx, rx) = channel::<(String, SuiteResult)>();
        let timer = Instant::now();
        let handle = tokio::task::spawn({
            let filter = filter.clone();
            async move { runner.test(&filter, tx, test_options).await }
        });

        let mut gas_report =
            self.gas_report.then(|| GasReport::new(config.gas_reports, config.gas_reports_ignore));

        // Build the trace decoder.
        let mut builder = CallTraceDecoderBuilder::new()
            .with_local_identifier_abis(&local_identifier)
            .with_verbosity(verbosity);
        // Signatures are of no value for gas reports.
        if !self.gas_report {
            builder = builder.with_signature_identifier(SignaturesIdentifier::new(
                Config::foundry_cache_dir(),
                config.offline,
            )?);
        }
        let mut decoder = builder.build();

        // We identify addresses if we're going to print *any* trace or gas report.
        let identify_addresses = verbosity >= 3 || self.gas_report || self.debug.is_some();

        let mut outcome = TestOutcome::empty(self.allow_failure);

        let mut any_test_failed = false;
        for (contract_name, suite_result) in rx {
            let tests = &suite_result.test_results;

            // Print suite header.
            println!();
            for warning in suite_result.warnings.iter() {
                eprintln!("{} {warning}", Paint::yellow("Warning:").bold());
            }
            if !tests.is_empty() {
                let len = tests.len();
                let tests = if len > 1 { "tests" } else { "test" };
                println!("Ran {len} {tests} for {contract_name}");
            }

            // Process individual test results, printing logs and traces when necessary.
            for (name, result) in tests {
                shell::println(result.short_result(name))?;

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

                // We shouldn't break out of the outer loop directly here so that we finish
                // processing the remaining tests and print the suite summary.
                any_test_failed |= result.status == TestStatus::Failure;

                if result.traces.is_empty() {
                    continue;
                }

                // Clear the addresses and labels from previous runs.
                decoder.clear_addresses();
                decoder
                    .labels
                    .extend(result.labeled_addresses.iter().map(|(k, v)| (*k, v.clone())));

                // Identify addresses and decode traces.
                let mut decoded_traces = Vec::with_capacity(result.traces.len());
                for (kind, arena) in &result.traces {
                    if identify_addresses {
                        decoder.identify(arena, &mut local_identifier);
                        decoder.identify(arena, &mut etherscan_identifier);
                    }

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

                    if should_include {
                        decoded_traces.push(render_trace_arena(arena, &decoder).await?);
                    }
                }

                if !decoded_traces.is_empty() {
                    shell::println("Traces:")?;
                    for trace in &decoded_traces {
                        shell::println(trace)?;
                    }
                }

                if let Some(gas_report) = &mut gas_report {
                    gas_report.analyze(&result.traces, &decoder).await;
                }
            }

            // Print suite summary.
            shell::println(suite_result.summary())?;

            // Add the suite result to the outcome.
            outcome.results.insert(contract_name, suite_result);

            // Stop processing the remaining suites if any test failed and `fail_fast` is set.
            if self.fail_fast && any_test_failed {
                break;
            }
        }
        let duration = timer.elapsed();

        trace!(target: "forge::test", len=outcome.results.len(), %any_test_failed, "done with results");

        outcome.decoder = Some(decoder);

        if let Some(gas_report) = gas_report {
            shell::println(gas_report.finalize())?;
        }

        if !outcome.results.is_empty() {
            shell::println(outcome.summary(duration))?;

            if self.summary {
                let mut summary_table = TestSummaryReporter::new(self.detailed);
                shell::println("\n\nTest Summary:")?;
                summary_table.print_summary(&outcome);
            }
        }

        // Reattach the task.
        if let Err(e) = handle.await {
            match e.try_into_panic() {
                Ok(payload) => std::panic::resume_unwind(payload),
                Err(e) => return Err(e.into()),
            }
        }

        Ok(outcome)
    }

    /// Returns the flattened [`FilterArgs`] arguments merged with [`Config`].
    pub fn filter(&self, config: &Config) -> ProjectPathsAwareFilter {
        self.filter.clone().merge_with_config(config)
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

        if let Some(etherscan_api_key) =
            self.etherscan_api_key.as_ref().filter(|s| !s.trim().is_empty())
        {
            dict.insert("etherscan_api_key".to_string(), etherscan_api_key.to_string().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// Lists all matching tests
fn list(
    runner: MultiContractRunner,
    filter: &ProjectPathsAwareFilter,
    json: bool,
) -> Result<TestOutcome> {
    let results = runner.list(filter);

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
    Ok(TestOutcome::empty(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_config::Chain;

    #[test]
    fn watch_parse() {
        let args: TestArgs = TestArgs::parse_from(["foundry-cli", "-vw"]);
        assert!(args.watch.watch.is_some());
    }

    #[test]
    fn fuzz_seed() {
        let args: TestArgs = TestArgs::parse_from(["foundry-cli", "--fuzz-seed", "0x10"]);
        assert!(args.fuzz_seed.is_some());
    }

    // <https://github.com/foundry-rs/foundry/issues/5913>
    #[test]
    fn fuzz_seed_exists() {
        let args: TestArgs =
            TestArgs::parse_from(["foundry-cli", "-vvv", "--gas-report", "--fuzz-seed", "0x10"]);
        assert!(args.fuzz_seed.is_some());
    }

    #[test]
    fn extract_chain() {
        let test = |arg: &str, expected: Chain| {
            let args = TestArgs::parse_from(["foundry-cli", arg]);
            assert_eq!(args.evm_opts.env.chain, Some(expected));
            let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
            assert_eq!(config.chain, Some(expected));
            assert_eq!(evm_opts.env.chain_id, Some(expected.id()));
        };
        test("--chain-id=1", Chain::mainnet());
        test("--chain-id=42", Chain::from_id(42));
    }
}
