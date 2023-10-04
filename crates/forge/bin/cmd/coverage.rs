use super::{install, test::FilterArgs};
use alloy_primitives::{Address, Bytes, U256};
use clap::{Parser, ValueEnum, ValueHint};
use ethers::{
    prelude::{
        artifacts::{Ast, CompactBytecode, CompactDeployedBytecode},
        Artifact, Project, ProjectCompileOutput,
    },
    solc::{artifacts::contract::CompactContractBytecode, sourcemap::SourceMap},
};
use eyre::{Context, Result};
use forge::{
    coverage::{
        analysis::SourceAnalyzer, anchors::find_anchors, ContractId, CoverageReport,
        CoverageReporter, DebugReporter, ItemAnchor, LcovReporter, SummaryReporter,
    },
    executor::{inspector::CheatsConfig, opts::EvmOpts},
    result::SuiteResult,
    revm::primitives::SpecId,
    utils::{build_ic_pc_map, ICPCMap},
    MultiContractRunnerBuilder, TestOptions,
};
use foundry_cli::{
    opts::CoreBuildArgs,
    p_println,
    utils::{LoadConfig, STATIC_FUZZ_SEED},
};
use foundry_common::{compile::ProjectCompiler, evm::EvmArgs, fs};
use foundry_config::{Config, SolcReq};
use foundry_utils::types::ToEthers;
use semver::Version;
use std::{collections::HashMap, path::PathBuf, sync::mpsc::channel};
use tracing::trace;
use yansi::Paint;

/// A map, keyed by contract ID, to a tuple of the deployment source map and the runtime source map.
type SourceMaps = HashMap<ContractId, (SourceMap, SourceMap)>;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(CoverageArgs, opts, evm_opts);

/// CLI arguments for `forge coverage`.
#[derive(Debug, Clone, Parser)]
pub struct CoverageArgs {
    /// The report type to use for coverage.
    ///
    /// This flag can be used multiple times.
    #[clap(long, value_enum, default_value = "summary")]
    report: Vec<CoverageReportKind>,

    /// Enable viaIR with minimum optimization
    ///
    /// This can fix most of the "stack too deep" errors while resulting a
    /// relatively accurate source map.
    #[clap(long)]
    ir_minimum: bool,

    /// The path to output the report.
    ///
    /// If not specified, the report will be stored in the root of the project.
    #[clap(
        long,
        short,
        value_hint = ValueHint::FilePath,
        value_name = "PATH"
    )]
    report_file: Option<PathBuf>,

    #[clap(flatten)]
    filter: FilterArgs,

    #[clap(flatten)]
    evm_opts: EvmArgs,

    #[clap(flatten)]
    opts: CoreBuildArgs,
}

impl CoverageArgs {
    pub async fn run(self) -> Result<()> {
        let (mut config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;

        // install missing dependencies
        if install::install_missing_dependencies(&mut config, self.build_args().silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
        }

        // Set fuzz seed so coverage reports are deterministic
        config.fuzz.seed = Some(U256::from_be_bytes(STATIC_FUZZ_SEED).to_ethers());

        let (project, output) = self.build(&config)?;
        p_println!(!self.opts.silent => "Analysing contracts...");
        let report = self.prepare(&config, output.clone())?;

        p_println!(!self.opts.silent => "Running tests...");
        self.collect(project, output, report, config, evm_opts).await
    }

    /// Builds the project.
    fn build(&self, config: &Config) -> Result<(Project, ProjectCompileOutput)> {
        // Set up the project
        let project = {
            let mut project = config.ephemeral_no_artifacts_project()?;

            if self.ir_minimum {
                // TODO: How to detect solc version if the user does not specify a solc version in
                // config  case1: specify local installed solc ?
                //  case2: mutliple solc versions used and  auto_detect_solc == true
                if let Some(SolcReq::Version(version)) = &config.solc {
                    if *version < Version::new(0, 8, 13) {
                        return Err(eyre::eyre!(
                            "viaIR with minimum optimization is only available in Solidity 0.8.13 and above."
                        ));
                    }
                }

                // print warning message
                p_println!(!self.opts.silent => "{}",
                Paint::yellow(
                concat!(
                "Warning! \"--ir-minimum\" flag enables viaIR with minimum optimization, which can result in inaccurate source mappings.\n",
                "Only use this flag as a workaround if you are experiencing \"stack too deep\" errors.\n",
                "Note that \"viaIR\" is only available in Solidity 0.8.13 and above.\n",
                "See more:\n",
                "https://github.com/foundry-rs/foundry/issues/3357\n"
                )));

                // Enable viaIR with minimum optimization
                // https://github.com/ethereum/solidity/issues/12533#issuecomment-1013073350
                // And also in new releases of solidity:
                // https://github.com/ethereum/solidity/issues/13972#issuecomment-1628632202
                project.solc_config.settings =
                    project.solc_config.settings.with_via_ir_minimum_optimization()
            } else {
                project.solc_config.settings.optimizer.disable();
                project.solc_config.settings.optimizer.runs = None;
                project.solc_config.settings.optimizer.details = None;
                project.solc_config.settings.via_ir = None;
            }

            project
        };

        let output = ProjectCompiler::default()
            .compile(&project)?
            .with_stripped_file_prefixes(project.root());

        Ok((project, output))
    }

    /// Builds the coverage report.
    #[tracing::instrument(name = "prepare coverage", skip_all)]
    fn prepare(&self, config: &Config, output: ProjectCompileOutput) -> Result<CoverageReport> {
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
    async fn collect(
        self,
        project: Project,
        output: ProjectCompileOutput,
        mut report: CoverageReport,
        config: Config,
        evm_opts: EvmOpts,
    ) -> Result<()> {
        let root = project.paths.root;

        // Build the contract runner
        let env = evm_opts.evm_env().await?;
        let mut runner = MultiContractRunnerBuilder::default()
            .initial_balance(evm_opts.initial_balance)
            .evm_spec(config.evm_spec_id())
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, env.clone()))
            .with_cheats_config(CheatsConfig::new(&config, &evm_opts))
            .with_test_options(TestOptions { fuzz: config.fuzz, ..Default::default() })
            .set_coverage(true)
            .build(root.clone(), output, env, evm_opts)?;

        // Run tests
        let known_contracts = runner.known_contracts.clone();
        let filter = self.filter;
        let (tx, rx) = channel::<(String, SuiteResult)>();
        let handle =
            tokio::task::spawn(
                async move { runner.test(filter, Some(tx), Default::default()).await },
            );

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
        let _ = handle.await;

        // Output final report
        for report_kind in self.report {
            match report_kind {
                CoverageReportKind::Summary => SummaryReporter::default().report(&report),
                CoverageReportKind::Lcov => {
                    match self.report_file {
                        Some(_) => {
                            return LcovReporter::new(&mut fs::create_file(
                                root.join(self.report_file.as_ref().unwrap()),
                            )?)
                            .report(&report)
                        }
                        None => {
                            return LcovReporter::new(&mut fs::create_file(root.join("lcov.info"))?)
                                .report(&report)
                        }
                    };
                }
                CoverageReportKind::Debug => DebugReporter.report(&report),
            }?;
        }
        Ok(())
    }

    /// Returns the flattened [`CoreBuildArgs`]
    pub fn build_args(&self) -> &CoreBuildArgs {
        &self.opts
    }
}

// TODO: HTML
#[derive(Debug, Clone, ValueEnum)]
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
            obj.link(&file, library, Address::ZERO.to_ethers());
        }
    }

    obj.object.resolve();
    obj.object.into_bytes().map(|o| o.0.into())
}

/// Helper function that will link references in unlinked bytecode to the 0 address.
///
/// This is needed in order to analyze the bytecode for contracts that use libraries.
fn dummy_link_deployed_bytecode(obj: CompactDeployedBytecode) -> Option<Bytes> {
    obj.bytecode.and_then(dummy_link_bytecode)
}
