use super::install;
use clap::Parser;
use eyre::{eyre, Result, Error};
use forge::{
    inspectors::CheatsConfig,
    result::SuiteResult,
    MultiContractRunnerBuilder, TestOptions, TestOptionsBuilder,
};
use foundry_cli::{
    init_progress,
    update_progress,
    opts::CoreBuildArgs,
    utils::{self, LoadConfig},
};
use foundry_common::{
    compile::{self, ProjectCompiler},
    evm::EvmArgs,
    shell::{self}
};
use foundry_compilers::{project_util::{copy_dir, TempProject}, report};
use foundry_config::{
    figment,
    figment::{
        value::{Dict, Map},
        Metadata, Profile, Provider,
    },
    get_available_profiles, Config,
};
use foundry_evm::opts::EvmOpts;
use futures::future::try_join_all;
use itertools::Itertools;
use std::{collections::BTreeMap, fs, sync::mpsc::channel, time::{Duration, Instant}, path::{PathBuf, Path}};
use yansi::Paint;
use foundry_evm_mutator::{Mutant, Mutator, MutatorConfigBuilder};
use crate::cmd::{
    test::{test, FilterArgs},
    mutate::mutation_summary::{
        MutationTestOutcome,
        MutantTestResult,
        MutationTestSuiteResult,
        MutationTestSummaryReporter
    }
};


mod filter;
pub use filter::*;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(MutateTestArgs, opts, evm_opts);

/// CLI arguments for `forge mutate`.
#[derive(Debug, Clone, Parser)]
#[clap(next_help_heading = "Mutation Test options")]
pub struct MutateTestArgs {
    /// Output Rutant test results in JSON format.
    #[clap(long, short, help_heading = "Display options")]
    json: bool,

    #[clap(flatten)]
    filter: MutationTestFilterArgs,

    /// Exit with code 0 even if a test fails.
    #[clap(long, env = "FORGE_ALLOW_FAILURE")]
    allow_failure: bool,

    /// Stop running mutation tests after the first failure
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
    #[clap(long, help_heading = "Display options", default_value_t = true)]
    pub summary: bool,

    /// Print detailed mutation test summary table
    #[clap(long, help_heading = "Display options")]
    pub detailed: bool,
}


impl MutationTestArgs {
    /// Returns the flattened [`CoreBuildArgs`].
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }

    pub async fn run(self) -> Result<MutationTestOutcome> {
        trace!(target: "forge::mutate", "executing mutation command");
        shell::set_shell(shell::Shell::from_args(self.opts.silent, self.json))?;
        println!("\n{}{}", 
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
        println!("{}\n", Paint::white("[1] Setting up and testing project...").bold());

        // let spinner = SpinnerReporter
        let (mut config, mut evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;
        // Fetch project filter
        let mut filter = self.filter(&config);

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
        // @TODO should we support the compilation match in tests
        let output = compile::suppress_compile(&project)?;

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

        let mut runner_builder = MultiContractRunnerBuilder::default()
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
        
        // We use empty filter to run the entire test suite during setup to ensure no failing tests
        let test_filter_args = FilterArgs {
            test_pattern: None,
            test_pattern_inverse: None,
            contract_pattern: None,
            contract_pattern_inverse: None,
            path_pattern: None,
            path_pattern_inverse: None,
        };
        
        let test_outcome = test(
            config.clone(),
            runner,
            0,
            test_filter_args.merge_with_config(&config),
            false,
            false,
            test_options,
            false,
            true,
            false,
            false
        ).await?;

        // ensure test outcome is ok
        // exit if any test is failing
        if test_outcome.failures().count() > 0 {
            test_outcome.ensure_ok()?;
        }

        println!("{}\n", Paint::white("[2] Generating mutants ...").bold());

        let mutator = MutatorConfigBuilder::new(
            project.solc.solc.clone(),
            config.optimizer,
            project.allowed_paths.paths().map(|x| x.to_owned()).collect_vec(),
            project.include_paths.paths().map(|x| x.to_owned()).collect(),
            config.remappings
        ).build(
            project_root.clone(),
            config.src.clone(), 
            output
        )?;

        if mutator.matching_function_count(&filter) == 0 {
            println!("\nNo functions match the provided pattern");
            println!("{}", filter.to_string());
            // Try to suggest a function when there's no match
            if let Some(ref function_pattern) = filter.args().function_pattern {
                let function_name = function_pattern.as_str();
                let candidates = mutator.get_functions(&filter);
                if let Some(suggestion) = utils::did_you_mean(function_name, candidates).pop() {
                    println!("\nDid you mean `{suggestion}`?");
                }

            }
        }
        // generate mutation
        let mutants_output = mutator.run_mutate(&config.src, filter.clone())?;
        // this is required for progress bar
        let mutants_len_iterator: Vec<&Mutant> = mutants_output.iter().flat_map(|(_, v)| v).collect();
        println!();
        println!("{}...\n", Paint::white("[3] Testing mutants").bold());

        let progress_bar = init_progress!(mutants_len_iterator, "Mutants");
        progress_bar.set_position(0);

        let mutant_test_suite_results: BTreeMap<String, MutationTestSuiteResult> = BTreeMap::from_iter(try_join_all(mutants_output.iter().map(
                |(file_name, mutants)| async {
                    let result = try_join_all(
                        mutants.iter().map(|mutant| {
                            test_mutant(
                                filter.clone(),
                                &project.root(),
                                &evm_opts,
                                mutant.clone()
                            )
                        })
                    ).await?;
                    update_progress!(progress_bar, mutants.len());
                    Ok::<(String, MutationTestSuiteResult), Error>((file_name.clone(), MutationTestSuiteResult::new(result)))
                }
            )
        ).await?.into_iter());

        if !progress_bar.is_finished() {
            progress_bar.finish();
        }

        let mutation_test_outcome = MutationTestOutcome::new(
            self.allow_failure,
            mutant_test_suite_results
        );

        println!();
        mutation_test_outcome.summary();

        if self.summary {
            let mut reporter = MutationTestSummaryReporter::new(self.detailed);
            println!();
            reporter.print_summary(&mutation_test_outcome);
        }

        Ok(mutation_test_outcome)
    }

    /// Returns the flattened [`MutationFilterArgs`] arguments merged with [`Config`].
    pub fn filter(&self, config: &Config) -> MutationProjectPathsAwareFilter {
        self.filter.merge_with_config(config)
    }
}

impl Provider for MutationTestArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Core Build Args - Mutation Test")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();

        // Override the fuzz and invariants run
        // We want fuzz and invariant tests to run once
        let mut fuzz_dict = Dict::default();
        fuzz_dict.insert("runs".to_string(),1.into());
        dict.insert("fuzz".to_string(), fuzz_dict.into());

        let mut invariant_dict = Dict::default();
        invariant_dict.insert("runs".to_string(), 1.into());
        dict.insert("invariant".to_string(), invariant_dict.into());

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

pub async fn test_mutant(
    mut filter: MutationProjectPathsAwareFilter,
    mutation_project_root: &Path,
    evm_opts: &EvmOpts,
    mutant: Mutant
) -> Result<MutantTestResult> {
    info!("testing mutants");    
    let start = Instant::now();

    // @TODO do test mode matching check here
    let filename = mutant.source.filename_as_str();
    let project = TempProject::dapptools()?;
    // copy project source code to temp dir
    copy_dir(mutation_project_root, &project.root())?;

    // get mutant source
    let mutant_contents = mutant.as_source_string().map_err(
        |err| eyre!("{:?}", err)
    )?;
    // setup file source root
    let file_source_root = project.root().join(filename);
    // Write Mutant contents to file in temp_directory
    fs::write(file_source_root.as_path(), mutant_contents)?;


    let mut config  = Config::load_with_root(&project.root());
    // appends the root dir to the config folder variables
    // it's important
    config = config.canonic();
    // override fuzz and invariant runs
    config.fuzz.runs = 1;
    config.invariant.runs  = 1;
    let project = config.project()?;
    let env = evm_opts.evm_env().await?;
    let output = project.compile()?;
    let project_root = &project.root();

    let toml = config.get_config_path();
    // println!("project_root {:?}", toml);
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

    let mut runner = runner_builder.build(
        project_root,
        output.clone(),
        env.clone(),
        evm_opts.clone(),
    )?;

    // mpsc channel for reporting test results
    let (tx, rx) = channel::<(String, SuiteResult)>();
    let handle =
        tokio::task::spawn(async move { runner.test(filter, Some(tx), test_options).await });

    let mut status : MutantTestStatus = MutantTestStatus::Survived;

    'outer: for (_, suite_result) in rx {
        // If there were any test failures that means the mutant
        // was killed so exit
        if suite_result.failures().count() > 0 {
            status = MutantTestStatus::Killed;
            break 'outer
        }

    }

    // @TODO if FAIL FAST if a mutant survives then throw ERROR
    let _results = handle.await?;
    trace!(target: "forge::mutate", "received results");
    
    Ok(MutantTestResult::new(start.elapsed(), mutant, status))

}

