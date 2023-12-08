use super::{
    install,
    test::{ProjectPathsAwareFilter, TestOutcome},
};
use crate::cmd::{
    mutate::summary::{
        MutantTestResult, MutationTestOutcome, MutationTestSuiteResult, MutationTestSummaryReporter,
    },
    test::{test, FilterArgs},
};
use clap::Parser;
use ethers_core::types::Filter;
use eyre::{eyre, Error, Result};
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
    compile::{self, ProjectCompiler},
    evm::EvmArgs,
    shell::{self, println},
    term::Spinner,
};
use foundry_compilers::{
    project_util::{copy_dir, TempProject},
    report, Project, ProjectCompileOutput, TestFileFilter,
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
use foundry_evm_mutator::{Mutant, Mutator, MutatorConfigBuilder};
use futures::future::try_join_all;
use itertools::Itertools;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::mpsc::channel,
    time::{Duration, Instant},
};
use yansi::Paint;

mod filter;
pub use filter::*;
mod summary;
pub use summary::*;
mod reporter;
pub use reporter::*;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(MutateTestArgs, opts, evm_opts);

// @TODO
// export
// command line output

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
            "\n{}{}",
            Paint::white("[.] Starting Mutation Test. "),
            Paint::white("Go grab a cup of coffee ☕, it's going to take a while").bold()
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
        println!();
        println!("{}\n", Paint::white("Compiling and testing project...").bold());

        // let spinner = SpinnerReporter
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
            &test_filter
        ).await?;

        // Ensure test outcome is ok, exit if any test is failing
        if test_outcome.failures().count() > 0 {
            test_outcome.ensure_ok()?;
        }

        println!();
        let spinner = MutateSpinnerReporter::new("Generating Mutants...\n");
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

        // @NOTE/@TODO This step is quite slow and expensive run time wise
        // This is because Gambit writes to disk multiple files and also general several mutants
        // per line "caught"
        // Improvements can be done to improve gambit performance
        trace!("Running gambit");
        let mutants_output = mutator.run_mutate(self.export, mutate_filter.clone())?;
        trace!("Finished running gambit");
        // this is required for progress bar
        let mutants_len_iterator: Vec<&Mutant> =
            mutants_output.iter().flat_map(|(_, v)| v).collect();
        // Finish spinner
        spinner.finish();

        println!();
        println!("[.] Testing Mutants...");
        let progress_bar = init_progress!(mutants_len_iterator, "Mutants");
        progress_bar.set_position(0);

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
        let mut mutant_test_suite_results: BTreeMap<String, MutationTestSuiteResult> =
            BTreeMap::new();
        let mut progress_bar_index = 0;
        for (file_name, mutants) in mutants_output.iter() {
            let mut mutant_test_results = vec![];
            let start = Instant::now();
            let (temp_project, config, project) = setup_mutant_dir(&project.root())?;
            for mutant in mutants.iter() {
                let result = test_mutant(
                    test_filter.clone(),
                    &temp_project,
                    &config,
                    &evm_opts,
                    mutant.clone(),
                    &project,
                )
                .await?;
                let mutant_survived = result.survived();
                mutant_test_results.push(result);
                progress_bar_index += 1;
                update_progress!(progress_bar, progress_bar_index);

                // if fail is enabled then we exit on the
                // first mutant we encounter that survived
                if self.fail_fast && mutant_survived {
                    break;
                }
            }
            let duration = start.elapsed();
            mutant_test_suite_results.insert(
                file_name.clone(),
                MutationTestSuiteResult::new(duration, mutant_test_results),
            );
        }

        // finish progress bar
        progress_bar.finish();

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
        test_filter: &ProjectPathsAwareFilter,
    ) -> Result<(TestOutcome, ProjectCompileOutput)> {
        let output = project.compile()?;
        output.assert_success();

        // Create test options from general project settings
        // and compiler output
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

        let runner = runner_builder.clone().build(
            project_root,
            output.clone(),
            env.clone(),
            evm_opts.clone(),
        )?;

        Ok((
            test(
                config.clone(),
                runner,
                0,
                test_filter.clone(),
                false,
                false,
                test_options,
                false,
                true,
                false,
                false,
            )
            .await?,
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

        // let mut mutate_dict = Dict::default();
        // mutate_dict.insert("export".into(), self.export.into());
        // dict.insert("mutate".into(), mutate_dict.into());

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

pub fn setup_mutant_dir(mutation_project_root: &Path) -> Result<(TempProject, Config, Project)> {
    info!("Setting up temp mutant project dir");
    let project = TempProject::dapptools()?;

    // copy project source code to temp dir
    copy_dir(mutation_project_root, &project.root())?;

    let temp_project_root = project.root();
    // load config for this temp project
    let mut config = Config::load_with_root(&temp_project_root);
    // appends the root dir to the config folder variables
    // it's important
    config = config.canonic_at(&project.root());
    // override fuzz and invariant runs
    config.fuzz.runs = 0;
    config.invariant.runs = 0;

    // We do not recompile the project here because the artifacts generated
    // have relative paths.
    // The project has been compiled in the test step so we can re-use the
    // artifacts generated
    let update_project = config.project()?;
    let c = update_project.compile()?;
    // c.
    // @TODO fix with_stripped_file_prefixes

    Ok((project, config, update_project))
}

pub async fn test_mutant(
    filter: ProjectPathsAwareFilter,
    temp_project: &TempProject,
    config: &Config,
    evm_opts: &EvmOpts,
    mutant: Mutant,
    project: &Project,
) -> Result<MutantTestResult> {
    info!("Testing Mutants");

    let start = Instant::now();
    // get mutant source
    // @TODO can i cache the step and edit the file in memory
    let mutant_contents = mutant.as_source_string().map_err(|err| eyre!("{:?}", err))?;
    // println!("{:?}", mutant_contents);
    // setup file source root
    let mutant_filename = mutant.source.filename_as_str();
    let file_source_root = temp_project.root().join(mutant_filename);
    // println!("{:?}", file_source_root.to_string_lossy());
    // Write Mutant contents to file in temp_directory
    fs::write(&file_source_root.clone().as_path(), mutant_contents)?;

    // let project = config.project()?;
    let env = evm_opts.evm_env().await?;
    // This should only recompile the changed file. It makes the compilation step very fast
    // This is quite expensive in terms of time taken
    // let output = project.compile_sparse(
    // move |path: &Path| {
    //     println!("{:?}", path.to_str());
    //     path.starts_with(&file_source_root) || path.ends_with(".t.sol")

    // }
    // )?;
    let output = project.compile()?;
    // println!("{:?}", output);

    let project_root = &project.root();
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

    // @TODO if FAIL FAST if a mutant survives then throw ERROR
    let _results = handle.await?;

    trace!(target: "forge::mutate", "received results");

    Ok(MutantTestResult::new(start.elapsed(), mutant, status))
}
