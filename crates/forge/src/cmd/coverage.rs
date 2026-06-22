use super::{install, test::TestArgs, watch::WatchArgs};
use crate::coverage::{
    BytecodeReporter, ContractId, CoverageReport, CoverageReporter, CoverageSummaryReporter,
    DebugReporter, ItemAnchor, LcovReporter,
    analysis::{SourceAnalysis, SourceFiles},
    anchors::find_anchors,
};
use alloy_primitives::{Address, Bytes, U256, map::HashMap};
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::{LoadConfig, STATIC_FUZZ_SEED};
use foundry_common::{compile::ProjectCompiler, errors::convert_solar_errors};
use foundry_compilers::{
    Artifact, ArtifactId, Project, ProjectCompileOutput, ProjectPathsConfig, VYPER_EXTENSIONS,
    artifacts::{CompactBytecode, CompactDeployedBytecode, sourcemap::SourceMap},
};
use foundry_config::{Config, CoverageConfig, CoverageReportKind, parse_lcov_version};
use foundry_evm::{core::ic::IcPcMap, opts::EvmOpts};
use globset::{Glob, GlobSetBuilder};
use rayon::prelude::*;
use semver::Version;
use std::path::{Path, PathBuf};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(CoverageArgs, test);

/// CLI arguments for `forge coverage`.
///
/// Most flags here have a corresponding `[profile.<name>.coverage]` config
/// option in `foundry.toml`. CLI flags take precedence over config; the helper
/// `resolve_with` merges them after the config is loaded.
#[derive(Parser)]
pub struct CoverageArgs {
    /// The report type to use for coverage.
    ///
    /// This flag can be used multiple times. Falls back to the
    /// `[profile.<name>.coverage] report` config value when not provided
    /// (default: `summary`).
    #[arg(long, value_enum)]
    report: Vec<CoverageReportKind>,

    /// The version of the LCOV "tracefile" format to use.
    ///
    /// Format: `MAJOR[.MINOR]`.
    ///
    /// Main differences:
    /// - `1.x`: The original v1 format.
    /// - `2.0`: Adds support for "line end" numbers for functions.
    /// - `2.2`: Changes the format of functions.
    ///
    /// Falls back to the `[profile.<name>.coverage] lcov_version` config value
    /// when not provided.
    #[arg(long = "lcov-version", value_parser = parse_lcov_version)]
    lcov_version_cli: Option<Version>,

    /// The resolved LCOV version to use after merging CLI and config values.
    #[arg(skip = Version::new(1, 0, 0))]
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
    #[arg(
        long,
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

    /// The coverage reporters to use. Constructed from the other fields.
    #[arg(skip)]
    reporters: Vec<Box<dyn CoverageReporter>>,

    /// Glob patterns of source files to exclude from the coverage report.
    /// Populated from `[profile.<name>.coverage] skip_files` after config is
    /// loaded; not exposed directly on the CLI.
    #[arg(skip)]
    skip_files: Vec<String>,

    #[command(flatten)]
    test: TestArgs,
}

impl CoverageArgs {
    pub async fn run(mut self) -> Result<()> {
        let (mut config, evm_opts) = self.load_config_and_evm_opts()?;

        // install missing dependencies
        if install::install_missing_dependencies(&mut config).await && config.auto_detect_remappings
        {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }

        // Default to a static fuzz seed so coverage reports are deterministic,
        // but allow the user to override it via `--fuzz-seed` or `[fuzz] seed` in config.
        if config.fuzz.seed.is_none() {
            config.fuzz.seed = Some(U256::from_be_bytes(STATIC_FUZZ_SEED));
        }

        // Merge CLI args with `[profile.<name>.coverage]` config values. CLI
        // flags take precedence; unset CLI flags fall back to the config.
        self.resolve_with(&config.coverage);

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

    /// Merge `[profile.<name>.coverage]` config values into this struct. CLI
    /// flags already set on `self` win; unset/false flags inherit from
    /// `config`.
    ///
    /// After this returns:
    /// - `self.report` is non-empty.
    /// - boolean flags reflect `cli || config` (CLI cannot disable a flag set to `true` in config;
    ///   this matches the pre-existing flag-only semantics where booleans defaulted to `false`).
    fn resolve_with(&mut self, config: &CoverageConfig) {
        if self.report.is_empty() {
            self.report.clone_from(&config.report);
        }
        self.lcov_version =
            self.lcov_version_cli.clone().unwrap_or_else(|| config.lcov_version.clone());
        if !self.ir_minimum {
            self.ir_minimum = config.ir_minimum;
        }
        if self.report_file.is_none() {
            self.report_file.clone_from(&config.report_file);
        }
        if !self.include_libs {
            self.include_libs = config.include_libs;
        }
        if !self.exclude_tests {
            self.exclude_tests = config.exclude_tests;
        }
        // Glob filters are additive — there's no CLI flag for these, so always
        // take from config.
        self.skip_files.clone_from(&config.skip_files);
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

        if self.ir_minimum {
            sh_warn!(
                "`--ir-minimum` enables `viaIR` with minimum optimization, \
                 which can result in inaccurate source mappings.\n\
                 Only use this flag as a workaround if you are experiencing \"stack too deep\" errors.\n\
                 Note that `viaIR` is production ready since Solidity 0.8.13 and above.\n\
                 See more: https://book.getfoundry.sh/guides/best-practices/stack-too-deep"
            )?;
        } else {
            sh_warn!(
                "optimizer settings and `viaIR` have been disabled for accurate coverage reports.\n\
                 If you encounter \"stack too deep\" errors, consider using `--ir-minimum` which \
                 enables `viaIR` with minimum optimization resolving most of the errors.\n\
                 See more: https://book.getfoundry.sh/guides/best-practices/stack-too-deep"
            )?;
        }

        config.disable_optimizations(&mut project, self.ir_minimum);

        let output = ProjectCompiler::new()
            .dynamic_test_linking(config.dynamic_test_linking)
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
            .run_tests(project_root, config, evm_opts, output, &filter, true, None)
            .await?;

        let known_contracts = outcome.known_contracts.as_ref().unwrap().clone();

        // Add hit data to the coverage report
        let data = outcome.results.values().flat_map(|suite| {
            let mut hits = Vec::new();
            for result in suite.test_results.values() {
                let Some(hit_maps) = result.line_coverage.as_ref() else { continue };
                for map in hit_maps.0.values() {
                    if let Some((id, _)) = known_contracts.find_by_deployed_code(map.bytecode()) {
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

        // Filter out ignored sources from the report.
        let file_root = filter.paths().root.as_path();
        if let Some(not_re) = &filter.args().coverage_pattern_inverse {
            report.retain_sources(|path: &Path| {
                let path = path.strip_prefix(file_root).unwrap_or(path);
                !not_re.is_match(&path.to_string_lossy())
            });
        }
        if !self.skip_files.is_empty() {
            let mut builder = GlobSetBuilder::new();
            for pattern in &self.skip_files {
                let glob = Glob::new(pattern).map_err(|e| {
                    eyre::eyre!("invalid glob in coverage.skip_files: '{pattern}': {e}")
                })?;
                builder.add(glob);
            }
            let set = builder
                .build()
                .map_err(|e| eyre::eyre!("failed to build coverage.skip_files glob set: {e}"))?;
            report.retain_sources(|path: &Path| {
                let path = path.strip_prefix(file_root).unwrap_or(path);
                !set.is_match(path)
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

    pub const fn is_watch(&self) -> bool {
        self.test.is_watch()
    }

    pub const fn watch(&self) -> &WatchArgs {
        &self.test.watch
    }
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

    #[test]
    fn resolve_lcov_version_uses_config_when_cli_absent() {
        let mut args = CoverageArgs::parse_from(["coverage"]);
        let config = CoverageConfig { lcov_version: Version::new(2, 2, 0), ..Default::default() };

        args.resolve_with(&config);

        assert_eq!(args.lcov_version, Version::new(2, 2, 0));
    }

    #[test]
    fn resolve_lcov_version_keeps_explicit_cli_default() {
        let mut args = CoverageArgs::parse_from(["coverage", "--lcov-version", "1"]);
        let config = CoverageConfig { lcov_version: Version::new(2, 2, 0), ..Default::default() };

        args.resolve_with(&config);

        assert_eq!(args.lcov_version, Version::new(1, 0, 0));
    }

    #[test]
    fn resolve_lcov_version_keeps_explicit_cli_value() {
        let mut args = CoverageArgs::parse_from(["coverage", "--lcov-version", "2"]);
        let config = CoverageConfig { lcov_version: Version::new(2, 2, 0), ..Default::default() };

        args.resolve_with(&config);

        assert_eq!(args.lcov_version, Version::new(2, 0, 0));
    }
}
