use super::{install, test::TestArgs, watch::WatchArgs};
use crate::coverage::{
    BytecodeReporter, ContractId, CoverageReport, CoverageReporter, CoverageSummaryReporter,
    DebugReporter, ItemAnchor, LcovReporter,
    analysis::{SourceAnalysis, SourceFiles},
    anchors::find_anchors,
};
use alloy_primitives::{Address, Bytes, U256, map::HashMap};
use clap::{Parser, ValueEnum, ValueHint};
use eyre::Result;
use foundry_cli::utils::{LoadConfig, STATIC_FUZZ_SEED};
use foundry_common::{compile::ProjectCompiler, errors::convert_solar_errors};
use foundry_compilers::{
    Artifact, ArtifactId, Project, ProjectCompileOutput, ProjectPathsConfig, VYPER_EXTENSIONS,
    artifacts::{CompactBytecode, CompactDeployedBytecode, sourcemap::SourceMap},
};
use foundry_config::Config;
use foundry_evm::{core::ic::IcPcMap, opts::EvmOpts};
use rayon::prelude::*;
use semver::{Version, VersionReq};
use std::path::{Path, PathBuf};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(CoverageArgs, test);

/// CLI arguments for `forge coverage`.
#[derive(Parser)]
pub struct CoverageArgs {
    /// The report type to use for coverage.
    ///
    /// This flag can be used multiple times.
    #[arg(long, value_enum, default_value = "summary")]
    report: Vec<CoverageReportKind>,

    /// The version of the LCOV "tracefile" format to use.
    ///
    /// Format: `MAJOR[.MINOR]`.
    ///
    /// Main differences:
    /// - `1.x`: The original v1 format.
    /// - `2.0`: Adds support for "line end" numbers for functions.
    /// - `2.2`: Changes the format of functions.
    #[arg(long, default_value = "1", value_parser = parse_lcov_version)]
    lcov_version: Version,

    /// Enable viaIR with minimum optimization
    ///
    /// This can fix most of the "stack too deep" errors while resulting a
    /// relatively accurate source map.
    #[arg(long)]
    ir_minimum: bool,

    /// The path to output the report.
    ///
    /// If not specified, the report will be stored in the root of the project.
    #[arg(long,
        short,
        value_hint = ValueHint::FilePath,
        value_name = "PATH"
    )]
    report_file: Option<PathBuf>,

    /// Whether to include libraries in the coverage report.
    #[arg(long)]
    include_libs: bool,

    /// Whether to exclude tests from the coverage report.
    #[arg(long)]
    exclude_tests: bool,

    /// Whether to use the experimental source instrumentation engine.
    #[arg(long, help_heading = "Experimental")]
    instrument_source: bool,

    /// The coverage reporters to use. Constructed from the other fields.
    #[arg(skip)]
    reporters: Vec<Box<dyn CoverageReporter>>,

    #[command(flatten)]
    test: TestArgs,
}

impl CoverageArgs {
    pub async fn run(mut self) -> Result<()> {
        let (mut config, evm_opts) = self.load_config_and_evm_opts()?;

        if self.instrument_source {
            return self.run_instrumented(config, evm_opts).await;
        }

        // install missing dependencies
        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }

        // Set fuzz seed so coverage reports are deterministic
        config.fuzz.seed = Some(U256::from_be_bytes(STATIC_FUZZ_SEED));

        let (paths, mut output) = {
            let (project, output) = self.build(&config)?;
            (project.paths, output)
        };

        self.populate_reporters(&paths.root);

        sh_println!("Analysing contracts...")?;
        let report = self.prepare(&paths, &mut output)?;

        sh_println!("Running tests...")?;
        self.collect(&paths.root, &output, report, config, evm_opts).await
    }

    /// Experimental source instrumentation mode.
    async fn run_instrumented(mut self, mut config: Config, evm_opts: EvmOpts) -> Result<()> {
        sh_println!("Experimental source instrumentation mode enabled.")?;

        // 1. Setup temp directory
        let temp_dir = tempfile::tempdir()?;
        let temp_root = temp_dir.path();

        // 2. Collect and instrument sources
        let mut coverage_items = Vec::new();
        let sess = solar::interface::Session::builder().with_stderr_emitter().build();

        let project = config.project()?;
        let source_paths = project.paths.input_files();

        let mut path_to_id: HashMap<PathBuf, usize> = HashMap::default();
        sess.enter_sequential(|| {
            for (id, path) in source_paths.iter().enumerate() {
                let rel_path = path.strip_prefix(&config.root)?;
                let target_path = temp_root.join(rel_path);
                if let Some(parent) = target_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                let content = std::fs::read_to_string(path)?;
                let mut instrumented_content = content.clone();

                let arena = solar::ast::Arena::new();
                let mut parser = solar::parse::Parser::from_source_code(
                    &sess,
                    &arena,
                    solar::interface::source_map::FileName::Real(path.clone()),
                    content,
                )
                .map_err(|e| {
                    eyre::eyre!("Failed to create parser for {:?}: {:?}", path, e)
                })?;

                match parser.parse_file() {
                    Ok(ast) => {
                        let mut instrumenter =
                            crate::coverage::instrument::Instrumenter::new(&sess, id as u32);
                        let _ =
                            solar::ast::visit::Visit::visit_source_unit(&mut instrumenter, &ast);
                        instrumenter.instrument(&mut instrumented_content);
                        coverage_items.extend(instrumenter.items);
                    }
                    Err(err) => {
                        sh_warn!("Failed to parse {:?}: {:?}", path, err)?;
                    }
                }

                instrumented_content.push_str(&format!(
                    "\n\ninterface VmCoverage_{} {{ function coverageHit(uint256,uint256) external pure; }}",
                    id
                ));
                std::fs::write(&target_path, instrumented_content)
                    .map_err(|e| eyre::eyre!("Failed to write instrumented file to {:?}: {}", target_path, e))?;
                path_to_id.insert(path.clone(), id);
            }
            Ok::<(), eyre::Error>(())
        })?;

        // 3. Update config to point to temp root
        let original_root = config.root.clone();
        config.root = temp_root.to_path_buf();
        config.src = temp_root.join(config.src.strip_prefix(&original_root)?);
        config.test = temp_root.join(config.test.strip_prefix(&original_root)?);
        config.script = temp_root.join(config.script.strip_prefix(&original_root)?);

        // 4. Build instrumented project
        let (_project, output) = self.build(&config)?;

        // 5. Prepare Report
        let mut report = CoverageReport::default();
        let version = output
            .output()
            .sources
            .sources_with_version()
            .next()
            .map(|(_, _, v)| v.clone())
            .unwrap_or_else(|| Version::new(0, 8, 0));

        for (path, &id) in &path_to_id {
            let rel_path = path.strip_prefix(&original_root).unwrap_or(path);
            report.add_source(version.clone(), id, rel_path.to_path_buf());
        }

        let analysis = SourceAnalysis::from_items(coverage_items);
        report.add_analysis(version, analysis);

        self.populate_reporters(&original_root);

        sh_println!("Running tests...")?;
        self.collect(&original_root, &output, report, config, evm_opts).await?;

        Ok(())
    }

    fn populate_reporters(&mut self, root: &Path) {
        self.reporters = self
            .report
            .iter()
            .map(|report_kind| match report_kind {
                CoverageReportKind::Summary => {
                    Box::<CoverageSummaryReporter>::default() as Box<dyn CoverageReporter>
                }
                CoverageReportKind::Lcov => {
                    let path =
                        root.join(self.report_file.as_deref().unwrap_or("lcov.info".as_ref()));
                    Box::new(LcovReporter::new(path, self.lcov_version.clone()))
                }
                CoverageReportKind::Bytecode => Box::new(BytecodeReporter::new(
                    root.to_path_buf(),
                    root.join("bytecode-coverage"),
                )),
                CoverageReportKind::Debug => Box::new(DebugReporter),
            })
            .collect::<Vec<_>>();
    }

    /// Builds the project.
    fn build(&self, config: &Config) -> Result<(Project, ProjectCompileOutput)> {
        let mut project = config.ephemeral_project()?;

        // If `via_ir` is enabled in the config, we should use `ir_minimum` to avoid stack too deep
        // errors and because disabling it might break compilation.
        let use_ir_minimum = self.ir_minimum || config.via_ir;

        if use_ir_minimum {
            if !self.ir_minimum && config.via_ir {
                sh_warn!(
                    "Enabling `--ir-minimum` automatically because `via_ir` is enabled in configuration.\n\
                     This enables `viaIR` with minimum optimization, which can result in inaccurate source mappings.\n\
                     See more: https://github.com/foundry-rs/foundry/issues/3357"
                )?;
            } else {
                sh_warn!(
                    "`--ir-minimum` enables `viaIR` with minimum optimization, \
                     which can result in inaccurate source mappings.\n\
                     Only use this flag as a workaround if you are experiencing \"stack too deep\" errors.\n\
                     Note that `viaIR` is production ready since Solidity 0.8.13 and above.\n\
                     See more: https://github.com/foundry-rs/foundry/issues/3357"
                )?;
            }
        } else {
            sh_warn!(
                "optimizer settings and `viaIR` have been disabled for accurate coverage reports.\n\
                 If you encounter \"stack too deep\" errors, consider using `--ir-minimum` which \
                 enables `viaIR` with minimum optimization resolving most of the errors"
            )?;
        }

        config.disable_optimizations(&mut project, use_ir_minimum);

        let output = ProjectCompiler::default()
            .compile(&project)?
            .with_stripped_file_prefixes(project.root());

        Ok((project, output))
    }

    /// Builds the coverage report.
    #[instrument(name = "Coverage::prepare", skip_all)]
    fn prepare(
        &self,
        project_paths: &ProjectPathsConfig,
        output: &mut ProjectCompileOutput,
    ) -> Result<CoverageReport> {
        let mut report = CoverageReport::default();

        output.parser_mut().solc_mut().compiler_mut().enter_mut(|compiler| {
            if compiler.gcx().stage() < Some(solar::config::CompilerStage::Lowering) {
                let _ = compiler.lower_asts();
            }
            convert_solar_errors(compiler.dcx())
        })?;
        let output = &*output;

        // Collect source files.
        let mut versioned_sources = HashMap::<Version, SourceFiles>::default();
        for (path, source_file, version) in output.output().sources.sources_with_version() {
            // Filter out vyper sources.
            if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| VYPER_EXTENSIONS.contains(&ext))
            {
                continue;
            }

            report.add_source(version.clone(), source_file.id as usize, path.clone());

            // Filter out libs dependencies and tests.
            if (!self.include_libs && project_paths.has_library_ancestor(path))
                || (self.exclude_tests && project_paths.is_test(path))
            {
                continue;
            }

            let path = project_paths.root.join(path);
            versioned_sources
                .entry(version.clone())
                .or_default()
                .sources
                .insert(source_file.id, path);
        }

        // Get source maps and bytecodes.
        let artifacts: Vec<ArtifactData> = output
            .artifact_ids()
            .par_bridge() // This parses source maps, so we want to run it in parallel.
            .filter_map(|(id, artifact)| {
                let source_id = report.get_source_id(id.version.clone(), id.source.clone())?;
                ArtifactData::new(&id, source_id, artifact)
            })
            .collect();

        // Add coverage items.
        for (version, sources) in &versioned_sources {
            let source_analysis = SourceAnalysis::new(sources, output)?;
            let anchors = artifacts
                .par_iter()
                .filter(|artifact| artifact.contract_id.version == *version)
                .map(|artifact| {
                    let creation_code_anchors = artifact.creation.find_anchors(&source_analysis);
                    let deployed_code_anchors = artifact.deployed.find_anchors(&source_analysis);
                    (artifact.contract_id.clone(), (creation_code_anchors, deployed_code_anchors))
                })
                .collect_vec_list();
            report.add_anchors(anchors.into_iter().flatten());
            report.add_analysis(version.clone(), source_analysis);
        }

        if self.reporters.iter().any(|reporter| reporter.needs_source_maps()) {
            report.add_source_maps(artifacts.into_iter().map(|artifact| {
                (artifact.contract_id, (artifact.creation.source_map, artifact.deployed.source_map))
            }));
        }

        Ok(report)
    }

    /// Runs tests, collects coverage data and generates the final report.
    #[instrument(name = "Coverage::collect", skip_all)]
    async fn collect(
        mut self,
        project_root: &Path,
        output: &ProjectCompileOutput,
        mut report: CoverageReport,
        config: Config,
        evm_opts: EvmOpts,
    ) -> Result<()> {
        let filter = self.test.filter(&config)?;
        let outcome = self
            .test
            .run_tests(
                project_root,
                config,
                evm_opts,
                output,
                &filter,
                !self.instrument_source,
                self.instrument_source,
            )
            .await?;

        // Add hit data to the coverage report
        if self.instrument_source {
            for suite in outcome.results.values() {
                for result in suite.test_results.values() {
                    if let Some(source_hits) = &result.source_coverage {
                        report.add_source_hit_maps(source_hits)?;
                    }
                }
            }
        } else {
            let known_contracts = outcome.runner.as_ref().unwrap().known_contracts.clone();
            let data = outcome.results.values().flat_map(|suite| {
                let mut hits = Vec::new();
                for result in suite.test_results.values() {
                    let Some(hit_maps) = &result.line_coverage else { continue };
                    for map in hit_maps.0.values() {
                        if let Some((id, _)) = known_contracts.find_by_deployed_code(map.bytecode())
                        {
                            hits.push((id, map, true));
                        } else if let Some((id, _)) =
                            known_contracts.find_by_creation_code(map.bytecode())
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
        }

        // Filter out ignored sources from the report.
        if let Some(not_re) = &filter.args().coverage_pattern_inverse {
            let file_root = filter.paths().root.as_path();
            report.retain_sources(|path: &Path| {
                let path = path.strip_prefix(file_root).unwrap_or(path);
                !not_re.is_match(&path.to_string_lossy())
            });
        }

        // Output final reports.
        self.report(&report)?;

        // Check for test failures after generating coverage report.
        // This ensures coverage data is written even when tests fail.
        outcome.ensure_ok(false)?;

        Ok(())
    }

    #[instrument(name = "Coverage::report", skip_all)]
    fn report(&mut self, report: &CoverageReport) -> Result<()> {
        for reporter in &mut self.reporters {
            let _guard = debug_span!("reporter.report", kind=%reporter.name()).entered();
            reporter.report(report)?;
        }
        Ok(())
    }

    pub fn is_watch(&self) -> bool {
        self.test.is_watch()
    }

    pub fn watch(&self) -> &WatchArgs {
        &self.test.watch
    }
}

/// Coverage reports to generate.
#[derive(Clone, Debug, Default, ValueEnum)]
pub enum CoverageReportKind {
    #[default]
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
    /// Since our line coverage inspector collects hit data using program counters, the anchors
    /// also need to be based on program counters.
    ic_pc_map: IcPcMap,
}

impl BytecodeData {
    fn new(source_map: SourceMap, bytecode: Bytes) -> Self {
        let ic_pc_map = IcPcMap::new(&bytecode);
        Self { source_map, bytecode, ic_pc_map }
    }

    pub fn find_anchors(&self, source_analysis: &SourceAnalysis) -> Vec<ItemAnchor> {
        find_anchors(&self.bytecode, &self.source_map, &self.ic_pc_map, source_analysis)
    }
}

fn parse_lcov_version(s: &str) -> Result<Version, String> {
    let vr = VersionReq::parse(&format!("={s}")).map_err(|e| e.to_string())?;
    let [c] = &vr.comparators[..] else {
        return Err("invalid version".to_string());
    };
    if c.op != semver::Op::Exact {
        return Err("invalid version".to_string());
    }
    if !c.pre.is_empty() {
        return Err("pre-releases are not supported".to_string());
    }
    Ok(Version::new(c.major, c.minor.unwrap_or(0), c.patch.unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcov_version() {
        assert_eq!(parse_lcov_version("0").unwrap(), Version::new(0, 0, 0));
        assert_eq!(parse_lcov_version("1").unwrap(), Version::new(1, 0, 0));
        assert_eq!(parse_lcov_version("1.0").unwrap(), Version::new(1, 0, 0));
        assert_eq!(parse_lcov_version("1.1").unwrap(), Version::new(1, 1, 0));
        assert_eq!(parse_lcov_version("1.11").unwrap(), Version::new(1, 11, 0));
    }
}
