use super::{
    install,
    test::{ProjectPathsAwareFilter, TestOutcome},
};
use crate::cmd::{
    mutate::summary::{
        MutantTestResult, MutationTestOutcome, MutationTestSuiteResult, MutationTestSummaryReporter,
    }
};
use clap::Parser;
use eyre::{eyre, Result};
use forge::{
    inspectors::CheatsConfig, result::SuiteResult, MultiContractRunnerBuilder, TestOptions,
    TestOptionsBuilder,
};
use foundry_cli::{
    init_progress,
    opts::CoreBuildArgs,
    update_progress,
    utils::{self, LoadConfig},
};
use foundry_common::{
    compile::ProjectCompiler,
    evm::EvmArgs,
    shell::{self}, term::MutatorSpinnerReporter
};
use foundry_compilers::{
    project_util::{copy_dir, TempProject},
    Project, ProjectCompileOutput,
};
use foundry_config::{
    figment,
    figment::{
        value::{Dict, Map},
        Metadata, Profile, Provider,
    },
    get_available_profiles, Config,
};
use foundry_evm::opts::EvmOpts;
use foundry_evm_mutator::{Mutant, MutatorConfigBuilder};
use futures::future::try_join_all;
use itertools::Itertools;
use std::{
    collections::BTreeMap,
    fs,
    path::{PathBuf},
    sync::mpsc::channel,
    time::{Duration, Instant},
};

mod filter;
pub use filter::*;
mod summary;
pub use summary::*;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(MutateTestArgs, opts, evm_opts);


/// CLI arguments for `forge mutate`.
#[derive(Debug, Clone, Parser)]
#[clap(next_help_heading = "Mutation Test options")]
pub struct MutateTestArgs {
    /// Output mutate results in JSON format.
    #[clap(long, short, help_heading = "Display options")]
    json: bool,

    #[clap(flatten)]
    filter: MutateFilterArgs,

    /// Exit with code 0 even if a test fails.
    #[clap(long, env = "FORGE_ALLOW_FAILURE")]
    allow_failure: bool,

    /// Stop running mutation tests after the first surviving mutation
    #[clap(long)]
    pub fail_fast: bool,

    /// List mutation tests instead of running them
    #[clap(long, short, help_heading = "Display options")]
    list: bool,

    #[clap(flatten)]
    evm_opts: EvmArgs,

    #[clap(flatten)]
    opts: CoreBuildArgs,

    /// Export generated mutants to a directory
    #[clap(long, default_value_t = false)]
    pub export: bool,

    /// Print mutation test summary table
    #[clap(long, help_heading = "Display options", default_value_t = false)]
    pub summary: bool,

    /// Print detailed mutation test summary table
    #[clap(long, help_heading = "Display options")]
    pub detailed: bool,
}

impl MutateTestArgs {
    /// Returns the flattened [`CoreBuildArgs`].
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }

    pub async fn run(self) -> Result<MutationTestOutcome> {
        trace!(target: "forge::mutate", "executing mutation command");
        shell::set_shell(shell::Shell::from_args(self.opts.silent, self.json))?;
        println!(
            "{}",
            "[⠆] Starting Mutation Test. Go grab a cup of coffee ☕, it's going to take a while"
        );
        self.execute_mutation_test().await
    }

    /// Executes mutation test for the project
    ///
    /// This will trigger the build process and run tests that match the configured filter.
    /// On success mutants will get be generated for functions matching configured filter.
    /// On success tests matching configured filter will be executed for all mutants.
    ///
    /// Returns the mutation test results for all matching functions
    pub async fn execute_mutation_test(self) -> Result<MutationTestOutcome> {
        let (mut config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;
        // Fetch project mutate and test filter
        let (mutate_filter, test_filter) = self.filter(&config);

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

        let (test_outcome, output) = self.compile_and_test_project(
            &project,
            &config,
            &evm_opts,
            test_filter.clone()
        ).await?;

        // Ensure test outcome is ok, exit if any test is failing
        if test_outcome.failures().count() > 0 {
            test_outcome.ensure_ok()?;
        }

        let mutator = MutatorConfigBuilder::new(
            project.solc.solc.clone(),
            config.optimizer,
            project.allowed_paths.paths().map(|x| x.to_owned()).collect_vec(),
            project.include_paths.paths().map(|x| x.to_owned()).collect(),
            config.remappings,
        )
        .build(config.src.clone(), output)?;

        if mutator.matching_function_count(&mutate_filter) == 0 {
            println!("\nNo functions match the provided pattern");
            println!("{}", mutate_filter.to_string());
            // Try to suggest a function when there's no match
            if let Some(ref function_pattern) = mutate_filter.args().function_pattern {
                let function_name = function_pattern.as_str();
                let candidates = mutator.get_function_names(&mutate_filter);
                if let Some(suggestion) = utils::did_you_mean(function_name, candidates).pop() {
                    println!("\nDid you mean `{suggestion}`?");
                }
                std::process::exit(0);
            }
        }

        let now = Instant::now();
        // init spinner
        let spinner = MutatorSpinnerReporter::spawn("Generating Mutants...".into());
        trace!("running gambit");
        let mutants_output = mutator.run_mutate(self.export, mutate_filter.clone())?;

        let elapsed = now.elapsed();
        trace!(?elapsed, "finished running gambit");
        drop(spinner);

        // this is bad
        let mutants_len_iterator: Vec<&Mutant> = mutants_output.iter()
            .flat_map(|(_, v)| v)
            .collect();

        println!(
            "Generated {} mutants in {:.2?}",
            mutants_len_iterator.len(), elapsed
        );
        
        // 
        // @Notice This is having a race condition on Fuzz and Invariant tests
        // let mutant_test_suite_results: BTreeMap<String, MutationTestSuiteResult> =
        // BTreeMap::from_iter(try_join_all(mutants_output.iter().map(         |(file_name, mutants)| async {
        //             let start = Instant::now();
        //             let result = try_join_all(
        //                 mutants.iter().map(|mutant| {
        //                     test_mutant(
        //                         filter.clone(),
        //                         &project.root(),
        //                         &evm_opts,
        //                         mutant.clone()
        //                     )
        //                 })
        //             ).await?;
        //             let duration = start.elapsed();
        //             update_progress!(progress_bar, mutants.len());
        //             Ok::<(String, MutationTestSuiteResult), Error>((file_name.clone(),
        // MutationTestSuiteResult::new(duration, result)))         }
        //     )
        // ).await?.into_iter());
        
        // this is required for progress bar
        println!("[⠆] Testing Mutants...");
        let progress_bar = init_progress!(mutants_len_iterator, "Mutants");
        progress_bar.set_position(0);

        let mut mutant_test_suite_results: BTreeMap<String, MutationTestSuiteResult> =
            BTreeMap::new();

        let mut progress_bar_index = 0;
        
        let mutation_project_root = project.root();
        for (contract_out_dir, contract_mutants) in mutants_output.into_iter() {
            let mut mutant_test_statuses: Vec<(Duration, MutantTestStatus)> = Vec::with_capacity(contract_mutants.len());
            let contract_mutants_start = Instant::now();

            // a file can have hundreds of mutants depending on design
            // we chunk there to prevent I/O exhaustion as we use
            // try_join_all which launches all the futures and polls
            let mut contract_mutants_iterator = contract_mutants.chunks(6);
            
            while let Some(mutant_chunks) = contract_mutants_iterator.next() {
                let mutant_data_iterator = mutant_chunks.iter().map(
                    |mutant| (
                        mutation_project_root.clone(),
                        mutant.source.filename_as_str(),
                        mutant.as_source_string().expect("Failed to read a file")
                    )
                );

                // we compile the projects here
                let mutant_project_and_compile_output = try_join_all(
                    mutant_data_iterator
                    .map(
                        |(root, file_name, mutant_contents)| tokio::task::spawn_blocking(
                            || setup_and_compile_mutant(root, file_name, mutant_contents )
                        )
                    )
                ).await?;

                let mut mutant_project_and_compile_output_iterator = mutant_project_and_compile_output.into_iter();
                // We run the tests serially because each mutant project could have a lot tests and 
                // running the test is parallelized. 
                // So not to have huge resource consumption we run tests serially.
                // Also, running tests is pretty fast
                while let Some(Ok((temp_project, mutant_compile_output, mutant_config))) = mutant_project_and_compile_output_iterator.next() {
                    let (duration, mutant_test_status) =  test_mutant(
                        test_filter.clone(),
                        temp_project,
                        mutant_config,
                        mutant_compile_output,
                        &evm_opts
                    ).await?;
                    
                    let mutant_survived = mutant_test_status == MutantTestStatus::Survived;

                    mutant_test_statuses.push((duration, mutant_test_status));
                    if self.fail_fast && mutant_survived {
                        break;
                    }

                    progress_bar_index += 1;
                    update_progress!(progress_bar, progress_bar_index);
                }
            }

            let mutant_test_results: Vec<_> =  mutant_test_statuses.into_iter().enumerate().map(|(index, (duration, status))| {
                let mutant = contract_mutants.get(index).expect("this should not throw");
                MutantTestResult::new(duration, mutant.clone(), status)
            }).collect();

            let contract_name = contract_out_dir.split(std::path::MAIN_SEPARATOR_STR)
                .nth(1)
                .ok_or(eyre!("Failed to parse contract name"))?;

            mutant_test_suite_results.insert(
                contract_name.to_string(),
                MutationTestSuiteResult::new(contract_mutants_start.elapsed(), mutant_test_results),
            );
        }

        // finish progress bar
        progress_bar.finish_and_clear();

        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &mutant_test_suite_results
                        .values()
                        .flat_map(|x| x.mutation_test_results())
                        .collect::<Vec<_>>()
                )?
            );
            return Ok(MutationTestOutcome::new(self.allow_failure, mutant_test_suite_results));
        }

        let mutation_test_outcome =
            MutationTestOutcome::new(self.allow_failure, mutant_test_suite_results);

        if self.summary {
            let mut reporter = MutationTestSummaryReporter::new(self.detailed);
            println!();
            reporter.print_summary(&mutation_test_outcome);
        }

        println!();
        println!("{}", mutation_test_outcome.summary());

        Ok(mutation_test_outcome)
    }

    pub async fn compile_and_test_project(
        &self,
        project: &Project,
        config: &Config,
        evm_opts: &EvmOpts,
        filter: ProjectPathsAwareFilter,
    ) -> Result<(TestOutcome, ProjectCompileOutput)> {
        let compiler = ProjectCompiler::default();
        let output =  compiler.compile(&project)?;
        // Create test options from general project settings
        // and compiler output
        let start = Instant::now();
        let test_reporter = MutatorSpinnerReporter::spawn("Running Project Tests...".into());
        let project_root = &project.paths.root;
        let toml = config.get_config_path();
        let profiles = get_available_profiles(toml)?;
        let env = evm_opts.evm_env().await?;

        let test_options: TestOptions = TestOptionsBuilder::default()
            .fuzz(config.fuzz)
            .invariant(config.invariant)
            .profiles(profiles)
            .build(&output, project_root)?;

        let runner_builder = MultiContractRunnerBuilder::default()
            .set_debug(false)
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
        
        let mut results = BTreeMap::new();
        // Set up test reporter channel
        let (tx, rx) = channel::<(String, SuiteResult)>();

        // Run tests
        let handle =
            tokio::task::spawn(async move { runner.test(filter.clone(), Some(tx), test_options).await });

        'outer: for (contract_name, suite_result) in rx {
            results.insert(contract_name.clone(), suite_result.clone());
            if suite_result.failures().count() > 0 {
                break 'outer
            }
        }
        let _results = handle.await?;

        // stop reporter
        drop(test_reporter);

        println!("Finished running project tests in {:2?}", start.elapsed());
        Ok((
            TestOutcome::new(results, false),
            output,
        ))
    }

    /// Returns the flattened [`MutateFilterArgs`] arguments merged with [`Config`].
    pub fn filter(
        &self,
        config: &Config,
    ) -> (MutationProjectPathsAwareFilter, ProjectPathsAwareFilter) {
        self.filter.merge_with_config(config)
    }
}

impl Provider for MutateTestArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Mutation Test: Core Build Args ")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();

        // Override the fuzz and invariants run
        // We do not want fuzz and invariant tests to run once so the test
        // setup is faster.
        let mut fuzz_dict = Dict::default();
        fuzz_dict.insert("runs".to_string(), 0.into());
        dict.insert("fuzz".to_string(), fuzz_dict.into());

        let mut invariant_dict = Dict::default();
        invariant_dict.insert("runs".to_string(), 0.into());
        dict.insert("invariant".to_string(), invariant_dict.into());

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// Creates a temp project from source project and compiles the project
pub fn setup_and_compile_mutant(
    mutation_project_root: PathBuf,
    mutant_file: String,
    mutant_contents: String 
) -> Result<(TempProject, ProjectCompileOutput, Config)> {
    let start = Instant::now();

    // we do not support hardhat style testing
    let temp_project = TempProject::dapptools()?;
    let temp_project_root = temp_project.root();

    // copy project source code to temp dir
    copy_dir(mutation_project_root, temp_project_root)?;

    // load config for this temp project
    let mut config = Config::load_with_root(temp_project_root);
    // appends the root dir to the config folder variables
    // it's important
    config = config.canonic_at(temp_project_root);
    // override fuzz and invariant runs
    config.fuzz.runs = 0;
    config.invariant.runs = 0;

    let mutant_file_path= temp_project_root.join(mutant_file);
    // Write Mutant contents to file in temp_directory
    fs::write(&mutant_file_path.as_path(), mutant_contents)?;

    debug!(
        duration = ?start.elapsed(),
        "compilation times",
    );
    
    // @TODO compile_sparse should be the preferred approach it makes the compilation step
    // quite fast and skips output for generated files. But it doesn't re-compile files that inherit the "dirty" file
    // At the moment there is a bug in the implementation.
    // let output = project.compile_sparse(
    // move |path: &Path| {
    //     println!("{:?}", path.to_str());
    //     path.starts_with(&file_source_root) || path.ends_with(".t.sol")
    // }
    // )?;
    let compile_output = config.project()?.compile().map_err(|_| eyre!("compilation failed"))?;

    Ok((temp_project, compile_output, config))
}

/// Runs mutation test for a mutation temp project.
/// returns on the first failed test suite
pub async fn test_mutant(
    filter: ProjectPathsAwareFilter,
    temp_project: TempProject,
    config: Config,
    output: ProjectCompileOutput,
    evm_opts: &EvmOpts,
) -> Result<(Duration, MutantTestStatus)> {
    trace!(target: "forge::mutate", "testing mutant");

    let start = Instant::now();
    let env = evm_opts.evm_env().await?;

    let project_root = temp_project.root();
    let toml = config.get_config_path();
    let profiles = get_available_profiles(toml)?;

    let test_options: TestOptions = TestOptionsBuilder::default()
        .fuzz(config.fuzz)
        .invariant(config.invariant)
        .profiles(profiles)
        .build(&output, project_root)?;

    let runner_builder = MultiContractRunnerBuilder::default()
        .set_debug(false)
        .initial_balance(evm_opts.initial_balance)
        .evm_spec(config.evm_spec_id())
        .sender(evm_opts.sender)
        .with_fork(evm_opts.get_fork(&config, env.clone()))
        .with_cheats_config(CheatsConfig::new(&config, evm_opts.clone()))
        .with_test_options(test_options.clone());

    let mut runner =
        runner_builder.build(project_root, output.clone(), env.clone(), evm_opts.clone())?;

    // mpsc channel for reporting test results
    let (tx, rx) = channel::<(String, SuiteResult)>();
    let handle =
        tokio::task::spawn(async move { runner.test(filter, Some(tx), test_options).await });

    let mut status: MutantTestStatus = MutantTestStatus::Survived;

    'outer: for (_, suite_result) in rx {
        // If there were any test failures that means the mutant
        // was killed so exit
        if suite_result.failures().count() > 0 {
            status = MutantTestStatus::Killed;
            break 'outer;
        }
    }

    let _results = handle.await?;

    trace!(target: "forge::mutate", "received mutant test results");

    Ok((start.elapsed(), status))
}