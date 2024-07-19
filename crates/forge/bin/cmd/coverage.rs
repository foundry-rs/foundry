use super::{install, test::TestArgs};
use alloy_primitives::{Address, Bytes, U256};
use clap::{Parser, ValueEnum, ValueHint};
use eyre::{Context, Result};
use forge::{
    coverage::{
        analysis::{SourceAnalysis, SourceAnalyzer, SourceFile, SourceFiles},
        anchors::find_anchors,
        BytecodeReporter, ContractId, CoverageReport, CoverageReporter, DebugReporter, ItemAnchor,
        LcovReporter, SummaryReporter,
    },
    opts::EvmOpts,
    utils::IcPcMap,
    MultiContractRunnerBuilder, TestOptions,
};
use foundry_cli::{
    p_println,
    utils::{LoadConfig, STATIC_FUZZ_SEED},
};
use foundry_common::{compile::ProjectCompiler, fs};
use foundry_compilers::{
    artifacts::{sourcemap::SourceMap, CompactBytecode, CompactDeployedBytecode},
    Artifact, ArtifactId, Project, ProjectCompileOutput,
};
use foundry_config::{Config, SolcReq};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use semver::Version;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use yansi::Paint;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(CoverageArgs, test);

/// CLI arguments for `forge coverage`.
#[derive(Clone, Debug, Parser)]
pub struct CoverageArgs {
    /// The report type to use for coverage.
    ///
    /// This flag can be used multiple times.
    #[arg(long, value_enum, default_value = "summary")]
    report: Vec<CoverageReportKind>,

    /// Enable viaIR with minimum optimization
    ///
    /// This can fix most of the "stack too deep" errors while resulting a
    /// relatively accurate source map.
    #[arg(long)]
    ir_minimum: bool,

    /// The path to output the report.
    ///
    /// If not specified, the report will be stored in the root of the project.
    #[arg(
        long,
        short,
        value_hint = ValueHint::FilePath,
        value_name = "PATH"
    )]
    report_file: Option<PathBuf>,

    /// Whether to include libraries in the coverage report.
    #[arg(long)]
    include_libs: bool,

    #[command(flatten)]
    test: TestArgs,
}

impl CoverageArgs {
    pub async fn run(self) -> Result<()> {
        let (mut config, evm_opts) = self.load_config_and_evm_opts_emit_warnings()?;

        // install missing dependencies
        if install::install_missing_dependencies(&mut config, self.test.build_args().silent) &&
            config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config();
        }

        // Set fuzz seed so coverage reports are deterministic
        config.fuzz.seed = Some(U256::from_be_bytes(STATIC_FUZZ_SEED));

        // Coverage analysis requires the Solc AST output.
        config.ast = true;

        let (project, output) = self.build(&config)?;
        p_println!(!self.test.build_args().silent => "Analysing contracts...");
        let report = self.prepare(&project, &output)?;

        p_println!(!self.test.build_args().silent => "Running tests...");
        self.collect(project, &output, report, Arc::new(config), evm_opts).await
    }

    /// Builds the project.
    fn build(&self, config: &Config) -> Result<(Project, ProjectCompileOutput)> {
        // Set up the project
        let mut project = config.create_project(false, false)?;
        if self.ir_minimum {
            // TODO: How to detect solc version if the user does not specify a solc version in
            // config  case1: specify local installed solc ?
            //  case2: multiple solc versions used and  auto_detect_solc == true
            if let Some(SolcReq::Version(version)) = &config.solc {
                if *version < Version::new(0, 8, 13) {
                    return Err(eyre::eyre!(
                            "viaIR with minimum optimization is only available in Solidity 0.8.13 and above."
                        ));
                }
            }

            // print warning message
            let msg = concat!(
                "Warning! \"--ir-minimum\" flag enables viaIR with minimum optimization, \
                 which can result in inaccurate source mappings.\n",
                "Only use this flag as a workaround if you are experiencing \"stack too deep\" errors.\n",
                "Note that \"viaIR\" is only available in Solidity 0.8.13 and above.\n",
                "See more: https://github.com/foundry-rs/foundry/issues/3357",
            ).yellow();
            p_println!(!self.test.build_args().silent => "{msg}");

            // Enable viaIR with minimum optimization
            // https://github.com/ethereum/solidity/issues/12533#issuecomment-1013073350
            // And also in new releases of solidity:
            // https://github.com/ethereum/solidity/issues/13972#issuecomment-1628632202
            project.settings.solc.settings =
                project.settings.solc.settings.with_via_ir_minimum_optimization()
        } else {
            project.settings.solc.optimizer.disable();
            project.settings.solc.optimizer.runs = None;
            project.settings.solc.optimizer.details = None;
            project.settings.solc.via_ir = None;
        }

        let output = ProjectCompiler::default()
            .compile(&project)?
            .with_stripped_file_prefixes(project.root());

        Ok((project, output))
    }

    /// Builds the coverage report.
    #[instrument(name = "prepare", skip_all)]
    fn prepare(&self, project: &Project, output: &ProjectCompileOutput) -> Result<CoverageReport> {
        let mut report = CoverageReport::default();

        // Collect source files.
        let project_paths = &project.paths;
        let mut versioned_sources = HashMap::<Version, SourceFiles<'_>>::new();
        for (path, source_file, version) in output.output().sources.sources_with_version() {
            report.add_source(version.clone(), source_file.id as usize, path.clone());

            // Filter out dependencies
            if !self.include_libs && project_paths.has_library_ancestor(path) {
                continue;
            }

            if let Some(ast) = &source_file.ast {
                let file = project_paths.root.join(path);
                trace!(root=?project_paths.root, ?file, "reading source file");

                let source = SourceFile {
                    ast,
                    source: fs::read_to_string(&file)
                        .wrap_err("Could not read source code for analysis")?,
                };
                versioned_sources
                    .entry(version.clone())
                    .or_default()
                    .sources
                    .insert(source_file.id as usize, source);
            }
        }

        // Get source maps and bytecodes
        let artifacts: Vec<ArtifactData> = output
            .artifact_ids()
            .par_bridge()
            .filter_map(|(id, artifact)| {
                let source_id = report.get_source_id(id.version.clone(), id.source.clone())?;
                ArtifactData::new(&id, source_id, artifact)
            })
            .collect();

        // Add coverage items
        for (version, sources) in &versioned_sources {
            let source_analysis = SourceAnalyzer::new(sources).analyze()?;

            // Build helper mapping used by `find_anchors`
            let mut items_by_source_id = FxHashMap::<_, Vec<_>>::with_capacity_and_hasher(
                source_analysis.items.len(),
                Default::default(),
            );

            for (item_id, item) in source_analysis.items.iter().enumerate() {
                items_by_source_id.entry(item.loc.source_id).or_default().push(item_id);
            }

            let anchors = artifacts
                .par_iter()
                .filter(|artifact| artifact.contract_id.version == *version)
                .map(|artifact| {
                    let creation_code_anchors =
                        artifact.creation.find_anchors(&source_analysis, &items_by_source_id);
                    let deployed_code_anchors =
                        artifact.deployed.find_anchors(&source_analysis, &items_by_source_id);
                    (artifact.contract_id.clone(), (creation_code_anchors, deployed_code_anchors))
                })
                .collect::<Vec<_>>();

            report.add_anchors(anchors);
            report.add_items(version.clone(), source_analysis.items);
        }

        report.add_source_maps(artifacts.into_iter().map(|artifact| {
            (artifact.contract_id, (artifact.creation.source_map, artifact.deployed.source_map))
        }));

        Ok(report)
    }

    /// Runs tests, collects coverage data and generates the final report.
    async fn collect(
        self,
        project: Project,
        output: &ProjectCompileOutput,
        mut report: CoverageReport,
        config: Arc<Config>,
        evm_opts: EvmOpts,
    ) -> Result<()> {
        let root = project.paths.root;
        let verbosity = evm_opts.verbosity;

        // Build the contract runner
        let env = evm_opts.evm_env().await?;
        let runner = MultiContractRunnerBuilder::new(config.clone())
            .initial_balance(evm_opts.initial_balance)
            .evm_spec(config.evm_spec_id())
            .sender(evm_opts.sender)
            .with_fork(evm_opts.get_fork(&config, env.clone()))
            .with_test_options(TestOptions {
                fuzz: config.fuzz.clone(),
                invariant: config.invariant.clone(),
                ..Default::default()
            })
            .set_coverage(true)
            .build(&root, output, env, evm_opts)?;

        let known_contracts = runner.known_contracts.clone();

        let filter = self.test.filter(&config);
        let outcome =
            self.test.run_tests(runner, config.clone(), verbosity, &filter, output).await?;

        outcome.ensure_ok()?;

        // Add hit data to the coverage report
        let data = outcome.results.iter().flat_map(|(_, suite)| {
            let mut hits = Vec::new();
            for result in suite.test_results.values() {
                let Some(hit_maps) = result.coverage.as_ref() else { continue };
                for map in hit_maps.0.values() {
                    if let Some((id, _)) = known_contracts.find_by_deployed_code(&map.bytecode) {
                        hits.push((id, map, true));
                    } else if let Some((id, _)) =
                        known_contracts.find_by_creation_code(&map.bytecode)
                    {
                        hits.push((id, map, false));
                    }
                }
            }
            hits
        });

        for (artifact_id, map, is_deployed_code) in data {
            if let Some(source_id) =
                report.get_source_id(artifact_id.version.clone(), artifact_id.source.clone())
            {
                report.add_hit_map(
                    &ContractId {
                        version: artifact_id.version.clone(),
                        source_id,
                        contract_name: artifact_id.name.as_str().into(),
                    },
                    map,
                    is_deployed_code,
                )?;
            }
        }

        // Filter out ignored sources from the report
        let file_pattern = filter.args().coverage_pattern_inverse.as_ref();
        let file_root = &filter.paths().root;
        report.filter_out_ignored_sources(|path: &Path| {
            file_pattern.map_or(true, |re| {
                !re.is_match(&path.strip_prefix(file_root).unwrap_or(path).to_string_lossy())
            })
        });

        // Output final report
        for report_kind in self.report {
            match report_kind {
                CoverageReportKind::Summary => SummaryReporter::default().report(&report),
                CoverageReportKind::Lcov => {
                    if let Some(report_file) = self.report_file {
                        return LcovReporter::new(&mut fs::create_file(root.join(report_file))?)
                            .report(&report)
                    } else {
                        return LcovReporter::new(&mut fs::create_file(root.join("lcov.info"))?)
                            .report(&report)
                    }
                }
                CoverageReportKind::Bytecode => {
                    let destdir = root.join("bytecode-coverage");
                    fs::create_dir_all(&destdir)?;
                    BytecodeReporter::new(root.clone(), destdir).report(&report)?;
                    Ok(())
                }
                CoverageReportKind::Debug => DebugReporter.report(&report),
            }?;
        }
        Ok(())
    }
}

// TODO: HTML
#[derive(Clone, Debug, ValueEnum)]
pub enum CoverageReportKind {
    Summary,
    Lcov,
    Debug,
    Bytecode,
}

/// Helper function that will link references in unlinked bytecode to the 0 address.
///
/// This is needed in order to analyze the bytecode for contracts that use libraries.
fn dummy_link_bytecode(mut obj: CompactBytecode) -> Option<Bytes> {
    let link_references = obj.link_references.clone();
    for (file, libraries) in link_references {
        for library in libraries.keys() {
            obj.link(&file, library, Address::ZERO);
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

pub struct ArtifactData {
    pub contract_id: ContractId,
    pub creation: BytecodeData,
    pub deployed: BytecodeData,
}

impl ArtifactData {
    pub fn new(id: &ArtifactId, source_id: usize, artifact: &impl Artifact) -> Option<Self> {
        Some(Self {
            contract_id: ContractId {
                version: id.version.clone(),
                source_id,
                contract_name: id.name.as_str().into(),
            },
            creation: BytecodeData::new(
                artifact.get_source_map()?.ok()?,
                artifact
                    .get_bytecode()
                    .and_then(|bytecode| dummy_link_bytecode(bytecode.into_owned()))?,
            ),
            deployed: BytecodeData::new(
                artifact.get_source_map_deployed()?.ok()?,
                artifact
                    .get_deployed_bytecode()
                    .and_then(|bytecode| dummy_link_deployed_bytecode(bytecode.into_owned()))?,
            ),
        })
    }
}

pub struct BytecodeData {
    source_map: SourceMap,
    bytecode: Bytes,
    /// The instruction counter to program counter mapping.
    ///
    /// The source maps are indexed by *instruction counters*, which are the indexes of
    /// instructions in the bytecode *minus any push bytes*.
    ///
    /// Since our coverage inspector collects hit data using program counters, the anchors
    /// also need to be based on program counters.
    ic_pc_map: IcPcMap,
}

impl BytecodeData {
    fn new(source_map: SourceMap, bytecode: Bytes) -> Self {
        let ic_pc_map = IcPcMap::new(&bytecode);
        Self { source_map, bytecode, ic_pc_map }
    }

    pub fn find_anchors(
        &self,
        source_analysis: &SourceAnalysis,
        items_by_source_id: &FxHashMap<usize, Vec<usize>>,
    ) -> Vec<ItemAnchor> {
        find_anchors(
            &self.bytecode,
            &self.source_map,
            &self.ic_pc_map,
            &source_analysis.items,
            items_by_source_id,
        )
    }
}
