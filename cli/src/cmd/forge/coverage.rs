//! Coverage command
use crate::{
    cmd::{
        forge::{build::CoreBuildArgs, test::Filter},
        Cmd, LoadConfig,
    },
    utils::{self, p_println, STATIC_FUZZ_SEED},
};
use clap::{AppSettings, ArgEnum, Parser};
use ethers::{
    abi::Address,
    prelude::{
        artifacts::{Ast, CompactBytecode, CompactDeployedBytecode},
        Artifact, Bytes, Project, ProjectCompileOutput, U256,
    },
    solc::{artifacts::contract::CompactContractBytecode, sourcemap::SourceMap},
};
use eyre::Context;
use forge::{
    coverage::{
        analysis::SourceAnalyzer, anchors::find_anchors, ContractId, CoverageReport,
        CoverageReporter, DebugReporter, ItemAnchor, LcovReporter, SummaryReporter,
    },
    executor::{inspector::CheatsConfig, opts::EvmOpts},
    result::SuiteResult,
    revm::SpecId,
    utils::{build_ic_pc_map, ICPCMap},
    MultiContractRunnerBuilder, TestOptions,
};
use foundry_common::{compile::ProjectCompiler, evm::EvmArgs, fs, ContractsByArtifact};
use foundry_config::Config;
use semver::Version;
use std::{collections::HashMap, sync::mpsc::channel, thread};
use tracing::trace;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(CoverageArgs, opts, evm_opts);

/// Generate coverage reports for your tests.
#[derive(Debug, Clone, Parser)]
#[clap(global_setting = AppSettings::DeriveDisplayOrder)]
pub struct CoverageArgs {
    #[clap(
        long,
        arg_enum,
        default_value = "summary",
        help = "The report type to use for coverage."
    )]
    report: CoverageReportKind,

    #[clap(flatten, next_help_heading = "TEST FILTERING")]
    filter: Filter,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    evm_opts: EvmArgs,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    opts: CoreBuildArgs,
}

impl CoverageArgs {
    /// Returns the flattened [`CoreBuildArgs`]
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }
}

impl Cmd for CoverageArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let (mut config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;

        // Set fuzz seed so coverage reports are deterministic
        config.fuzz.seed = Some(U256::from_big_endian(&STATIC_FUZZ_SEED));

        let (project, output) = self.build(&config)?;
        p_println!(!self.opts.silent => "Analysing contracts...");
        let report = self.prepare(&config, output.clone())?;

        p_println!(!self.opts.silent => "Running tests...");
        self.collect(project, output, report, config, evm_opts)
    }
}

/// A map, keyed by contract ID, to a tuple of the deployment source map and the runtime source map.
type SourceMaps = HashMap<ContractId, (SourceMap, SourceMap)>;

// The main flow of the command itself
impl CoverageArgs {
    /// Builds the project.
    fn build(&self, config: &Config) -> eyre::Result<(Project, ProjectCompileOutput)> {
        // Set up the project
        let project = {
            let mut project = config.ephemeral_no_artifacts_project()?;

            // Disable the optimizer for more accurate source maps
            project.solc_config.settings.optimizer.disable();

            project
        };

        let output = ProjectCompiler::default()
            .compile(&project)?
            .with_stripped_file_prefixes(project.root());

        Ok((project, output))
    }

    /// Builds the coverage report.
    #[tracing::instrument(name = "prepare coverage", skip_all)]
    fn prepare(
        &self,
        config: &Config,
        output: ProjectCompileOutput,
    ) -> eyre::Result<CoverageReport> {
        let project_paths = config.project_paths();

        // Extract artifacts
        let (artifacts, sources) = output.into_artifacts_with_sources();
        let mut report = CoverageReport::default();

        // Collect ASTs and sources
        let mut versioned_asts: HashMap<Version, HashMap<usize, Ast>> = HashMap::new();
        let mut versioned_sources: HashMap<Version, HashMap<usize, String>> = HashMap::new();
        for (path, mut source_file, version) in sources.into_sources_with_version() {
            // Filter out dependencies
            if project_paths.has_library_ancestor(std::path::Path::new(&path)) {
                continue
            }

            if let Some(ast) = source_file.ast.take() {
                versioned_asts
                    .entry(version.clone())
                    .or_default()
                    .insert(source_file.id as usize, ast);

                let file = project_paths.root.join(&path);
                trace!(root=?project_paths.root, ?file, "reading source file");

                versioned_sources.entry(version.clone()).or_default().insert(
                    source_file.id as usize,
                    fs::read_to_string(&file)
                        .wrap_err("Could not read source code for analysis")?,
                );
                report.add_source(version, source_file.id as usize, path);
            }
        }

        // Get source maps and bytecodes
        let (source_maps, bytecodes): (SourceMaps, HashMap<ContractId, (Bytes, Bytes)>) = artifacts
            .into_iter()
            .map(|(id, artifact)| (id, CompactContractBytecode::from(artifact)))
            .filter_map(|(id, artifact)| {
                let contract_id = ContractId {
                    version: id.version.clone(),
                    source_id: *report
                        .get_source_id(id.version, id.source.to_string_lossy().to_string())?,
                    contract_name: id.name,
                };
                let source_maps = (
                    contract_id.clone(),
                    (
                        artifact.get_source_map()?.ok()?,
                        artifact
                            .get_deployed_bytecode()
                            .as_ref()?
                            .bytecode
                            .as_ref()?
                            .source_map()?
                            .ok()?,
                    ),
                );
                let bytecodes = (
                    contract_id,
                    (
                        artifact
                            .get_bytecode()
                            .and_then(|bytecode| dummy_link_bytecode(bytecode.into_owned()))?,
                        artifact.get_deployed_bytecode().and_then(|bytecode| {
                            dummy_link_deployed_bytecode(bytecode.into_owned())
                        })?,
                    ),
                );

                Some((source_maps, bytecodes))
            })
            .unzip();

        // Build IC -> PC mappings
        //
        // The source maps are indexed by *instruction counters*, which are the indexes of
        // instructions in the bytecode *minus any push bytes*.
        //
        // Since our coverage inspector collects hit data using program counters, the anchors also
        // need to be based on program counters.
        // TODO: Index by contract ID
        let ic_pc_maps: HashMap<ContractId, (ICPCMap, ICPCMap)> = bytecodes
            .iter()
            .map(|(id, bytecodes)| {
                // TODO: Creation bytecode as well
                (
                    id.clone(),
                    (
                        build_ic_pc_map(SpecId::LATEST, bytecodes.0.as_ref()),
                        build_ic_pc_map(SpecId::LATEST, bytecodes.1.as_ref()),
                    ),
                )
            })
            .collect();

        // Add coverage items
        for (version, asts) in versioned_asts.into_iter() {
            let source_analysis = SourceAnalyzer::new(
                version.clone(),
                asts,
                versioned_sources.remove(&version).ok_or_else(|| {
                    eyre::eyre!(
                        "File tree is missing source code, cannot perform coverage analysis"
                    )
                })?,
            )?
            .analyze()?;
            let anchors: HashMap<ContractId, Vec<ItemAnchor>> = source_analysis
                .contract_items
                .iter()
                .filter_map(|(contract_id, item_ids)| {
                    // TODO: Creation source map/bytecode as well
                    Some((
                        contract_id.clone(),
                        find_anchors(
                            &bytecodes.get(contract_id)?.1,
                            &source_maps.get(contract_id)?.1,
                            &ic_pc_maps.get(contract_id)?.1,
                            item_ids,
                            &source_analysis.items,
                        ),
                    ))
                })
                .collect();
            report.add_items(version, source_analysis.items);
            report.add_anchors(anchors);
        }

        Ok(report)
    }

    /// Runs tests, collects coverage data and generates the final report.
    fn collect(
        self,
        project: Project,
        output: ProjectCompileOutput,
        mut report: CoverageReport,
        config: Config,
        evm_opts: EvmOpts,
    ) -> eyre::Result<()> {
        let root = project.paths.root;

        // Build the contract runner
        let evm_spec = utils::evm_spec(&config.evm_version);
        let env = evm_opts.evm_env_blocking()?;
        let mut runner = MultiContractRunnerBuilder::default()
            .initial_balance(evm_opts.initial_balance)
            .evm_spec(evm_spec)
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, env.clone()))
            .with_cheats_config(CheatsConfig::new(&config, &evm_opts))
            .with_test_options(TestOptions { fuzz: config.fuzz, ..Default::default() })
            .set_coverage(true)
            .build(root.clone(), output, env, evm_opts)?;

        // Run tests
        let known_contracts = ContractsByArtifact(runner.known_contracts.clone());
        let (tx, rx) = channel::<(String, SuiteResult)>();
        let handle =
            thread::spawn(move || runner.test(&self.filter, Some(tx), Default::default()).unwrap());

        // Add hit data to the coverage report
        for (artifact_id, hits) in rx
            .into_iter()
            .flat_map(|(_, suite)| suite.test_results.into_values())
            .filter_map(|mut result| result.coverage.take())
            .flat_map(|hit_maps| {
                hit_maps.0.into_values().filter_map(|map| {
                    Some((known_contracts.find_by_code(map.bytecode.as_ref())?.0, map))
                })
            })
        {
            // TODO: Note down failing tests
            if let Some(source_id) = report.get_source_id(
                artifact_id.version.clone(),
                artifact_id.source.to_string_lossy().to_string(),
            ) {
                let source_id = *source_id;
                // TODO: Distinguish between creation/runtime in a smart way
                report.add_hit_map(
                    &ContractId {
                        version: artifact_id.version.clone(),
                        source_id,
                        contract_name: artifact_id.name.clone(),
                    },
                    &hits,
                );
            }
        }

        // Reattach the thread
        let _ = handle.join();

        // Output final report
        match self.report {
            CoverageReportKind::Summary => SummaryReporter::default().report(report),
            // TODO: Sensible place to put the LCOV file
            CoverageReportKind::Lcov => {
                LcovReporter::new(&mut fs::create_file(root.join("lcov.info"))?).report(report)
            }
            CoverageReportKind::Debug => DebugReporter::default().report(report),
        }
    }
}

// TODO: HTML
#[derive(Debug, Clone, ArgEnum)]
pub enum CoverageReportKind {
    Summary,
    Lcov,
    Debug,
}

/// Helper function that will link references in unlinked bytecode to the 0 address.
///
/// This is needed in order to analyze the bytecode for contracts that use libraries.
fn dummy_link_bytecode(mut obj: CompactBytecode) -> Option<Bytes> {
    let link_references = obj.link_references.clone();
    for (file, libraries) in link_references {
        for library in libraries.keys() {
            obj.link(&file, library, Address::zero());
        }
    }

    obj.object.resolve();
    obj.object.into_bytes()
}

/// Helper function that will link references in unlinked bytecode to the 0 address.
///
/// This is needed in order to analyze the bytecode for contracts that use libraries.
fn dummy_link_deployed_bytecode(obj: CompactDeployedBytecode) -> Option<Bytes> {
    obj.bytecode.and_then(dummy_link_bytecode)
}
