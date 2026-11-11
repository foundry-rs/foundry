use super::{
    test::{ProjectPathsAwareFilter, TestArgs, TestExecutionOptions},
    watch::WatchArgs,
};
use crate::coverage::{
    BytecodeReporter, ContractId, CoverageAttributionReporter, CoverageReport, CoverageReporter,
    CoverageSummaryReporter, DebugReporter, GapsReporter, ItemAnchors, LcovReporter,
    ResolvedHitMap, ResolvedHitMaps,
    analysis::{SourceAnalysis, SourceFiles},
    anchors::{find_anchors, find_execution_anchors},
};
use alloy_json_abi::StateMutability;
use alloy_primitives::{
    Address, B256, Bytes, U256, keccak256,
    map::{HashMap, HashSet},
};
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::{FoundryPathExt, LoadConfig, STATIC_FUZZ_SEED};
use foundry_common::{
    ContractsByArtifactBuilder, TestFilter, compile::ProjectCompiler, errors::convert_solar_errors,
    version::SHORT_VERSION,
};
use foundry_compilers::{
    Artifact, ArtifactId, Graph, Project, ProjectCompileOutput, ProjectPathsConfig,
    VYPER_EXTENSIONS,
    artifacts::{CompactBytecode, CompactDeployedBytecode, Source, sourcemap::SourceMap},
    cache::SOLIDITY_FILES_CACHE_FILENAME,
    compilers::{
        Language,
        multi::{MultiCompilerLanguage, MultiCompilerParser},
    },
    error::SolcError,
    utils::source_files_iter,
};
use foundry_config::{
    Config, CoverageConfig, CoverageReportKind, InlineConfig, parse_lcov_version,
};
use foundry_evm::{core::ic::IcPcMap, opts::EvmOpts};
use globset::{Glob, GlobSetBuilder};
use rayon::prelude::*;
use semver::Version;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(CoverageArgs, test);

/// CLI arguments for `forge coverage`.
///
/// Most flags here have a corresponding `[profile.<name>.coverage]` config
/// option in `foundry.toml`. CLI flags take precedence over config; the helper
/// `resolve` merges them after the config is loaded.
#[derive(Parser)]
#[command(after_long_help = r#"Source attribution:
  Coverage follows compiler source maps. Inherited modifier code is reported under the
  source where the modifier is declared. Dependency sources are excluded by default;
  use `--include-libs` to include their coverage.

Compatibility:
  `forge coverage` supports test filters and `--watch`, but not test-only output or
  execution modes such as `--json`, `--junit`, `--list`, `--debug`, flame profiles,
  symbolic artifact replay, showmap replay, brutalization, or mutation testing. Use
  `--report lcov` for interoperable coverage data, `--report attribution` for
  Foundry's per-test JSON attribution report, or `--report gaps` for a JSON list of
  uncovered source items."#)]
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

    /// Enable viaIR with minimum optimization
    ///
    /// This can fix most of the "stack too deep" errors while resulting a
    /// relatively accurate source map.
    #[arg(long)]
    ir_minimum: bool,

    /// The path to output the report.
    ///
    /// Used only when a single file report is requested. If not specified, the
    /// report will be stored in the root of the project.
    #[arg(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "PATH"
    )]
    report_file: Option<PathBuf>,

    /// Include dependency sources in the coverage report.
    #[arg(long)]
    include_libs: bool,

    /// Whether to exclude tests from the coverage report.
    #[arg(long)]
    exclude_tests: bool,

    #[command(flatten)]
    test: TestArgs,
}

/// The coverage settings of a run: the CLI flags merged with the
/// `[profile.<name>.coverage]` config values by [`CoverageArgs::resolve`].
#[derive(Debug)]
struct CoverageSettings {
    /// The report types to generate. Never empty.
    report: Vec<CoverageReportKind>,
    /// The version of the LCOV "tracefile" format to use.
    lcov_version: Version,
    /// Whether to enable viaIR with minimum optimization.
    ir_minimum: bool,
    /// The path to output a single file report to.
    report_file: Option<PathBuf>,
    /// Whether to include dependency sources in the coverage report.
    include_libs: bool,
    /// Whether to exclude tests from the coverage report.
    exclude_tests: bool,
    /// Glob patterns of source files to exclude from the coverage report.
    skip_files: Vec<String>,
}

impl CoverageSettings {
    fn report_path(&self, root: &Path, default_file_name: &str) -> PathBuf {
        let report_file =
            (self.file_report_count() == 1).then_some(self.report_file.as_deref()).flatten();
        root.join(report_file.unwrap_or_else(|| Path::new(default_file_name)))
    }

    fn file_report_count(&self) -> usize {
        let has_lcov = self.report.iter().any(|kind| matches!(kind, CoverageReportKind::Lcov));
        let has_attribution =
            self.report.iter().any(|kind| matches!(kind, CoverageReportKind::Attribution));
        let has_gaps = self.report.iter().any(|kind| matches!(kind, CoverageReportKind::Gaps));
        usize::from(has_lcov) + usize::from(has_attribution) + usize::from(has_gaps)
    }

    /// The reporters that write the requested reports, except the attribution report, which
    /// needs the test outcome.
    fn reporters(&self, root: &Path) -> Vec<Box<dyn CoverageReporter>> {
        self.report
            .iter()
            .filter_map(|report_kind| match report_kind {
                CoverageReportKind::Summary => {
                    Some(Box::<CoverageSummaryReporter>::default() as Box<dyn CoverageReporter>)
                }
                CoverageReportKind::Lcov => {
                    let path = self.report_path(root, "lcov.info");
                    Some(Box::new(LcovReporter::new(path, self.lcov_version.clone())))
                }
                CoverageReportKind::Bytecode => Some(Box::new(BytecodeReporter::new(
                    root.to_path_buf(),
                    root.join("bytecode-coverage"),
                ))),
                CoverageReportKind::Debug => Some(Box::new(DebugReporter)),
                CoverageReportKind::Attribution => None,
                CoverageReportKind::Gaps => {
                    let path = self.report_path(root, "coverage-gaps.json");
                    Some(Box::new(GapsReporter::new(root.to_path_buf(), path)))
                }
            })
            .collect()
    }
}

impl CoverageArgs {
    pub(crate) fn ensure_mode_compatible(&self) -> Result<()> {
        self.test.ensure_coverage_mode_compatible()
    }

    pub async fn run(self) -> Result<()> {
        self.ensure_mode_compatible()?;

        let (mut config, evm_opts) = self.load_config_and_evm_opts()?;

        // install missing dependencies
        self.install_missing_dependencies(&mut config)?;

        // Default to a static fuzz seed so coverage reports are deterministic,
        // but allow the user to override it via `--fuzz-seed` or `[fuzz] seed` in config.
        if config.fuzz.seed.is_none() {
            config.fuzz.seed = Some(U256::from_be_bytes(STATIC_FUZZ_SEED));
        }

        // Merge CLI args with `[profile.<name>.coverage]` config values. CLI
        // flags take precedence; unset CLI flags fall back to the config.
        let settings = self.resolve(&config.coverage);
        let filter = self.test.filter(&config)?;

        let (paths, mut output) = {
            let (project, output) = Self::build(&config, &filter, settings.ir_minimum)?;
            (project.paths, output)
        };

        if settings.report_file.is_some() && settings.file_report_count() > 1 {
            sh_warn!(
                "`--report-file` is ignored when multiple file reports are requested; \
                 each report will use its default output path"
            )?;
        }

        let mut reporters = settings.reporters(&paths.root);

        sh_println!("Analysing contracts...")?;
        let report = Self::prepare(&settings, &reporters, &paths, &mut output)?;

        sh_println!("Running tests...")?;
        self.collect(
            &settings,
            &mut reporters,
            &paths.root,
            &output,
            report,
            config,
            evm_opts,
            filter,
        )
        .await
    }

    /// Merges `[profile.<name>.coverage]` config values with the CLI flags. CLI
    /// flags that are set win; unset/false flags inherit from `config`.
    ///
    /// In the result:
    /// - `report` is non-empty.
    /// - boolean flags reflect `cli || config` (CLI cannot disable a flag set to `true` in config;
    ///   this matches the pre-existing flag-only semantics where booleans defaulted to `false`).
    fn resolve(&self, config: &CoverageConfig) -> CoverageSettings {
        CoverageSettings {
            report: if self.report.is_empty() {
                config.report.clone()
            } else {
                self.report.clone()
            },
            lcov_version: self
                .lcov_version_cli
                .clone()
                .unwrap_or_else(|| config.lcov_version.clone()),
            ir_minimum: self.ir_minimum || config.ir_minimum,
            report_file: self.report_file.clone().or_else(|| config.report_file.clone()),
            include_libs: self.include_libs || config.include_libs,
            exclude_tests: self.exclude_tests || config.exclude_tests,
            // Glob filters are additive — there's no CLI flag for these, so always
            // take from config.
            skip_files: config.skip_files.clone(),
        }
    }

    /// Builds the project.
    fn build(
        config: &Config,
        filter: &ProjectPathsAwareFilter,
        ir_minimum: bool,
    ) -> Result<(Project, ProjectCompileOutput)> {
        let mut project = config.ephemeral_project()?;
        // A contended or unavailable cache retains ephemeral compilation. Keep the lock until
        // artifacts and build contexts have been loaded or published together.
        let cache = (config.cache && !config.deny.warnings())
            .then(|| config.coverage_cache_path())
            .flatten()
            .and_then(|path| {
                fs::create_dir_all(path.parent()?).ok()?;
                let lock = Config::lock_coverage_cache(&path).ok()?;
                match fs::create_dir(&path) {
                    Ok(()) => {
                        if fs::File::create_new(path.join(Config::COVERAGE_CACHE_MARKER)).is_err() {
                            let _ = fs::remove_dir(&path);
                            return None;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        if !path.join(Config::COVERAGE_CACHE_MARKER).is_file() {
                            return None;
                        }
                    }
                    Err(_) => return None,
                }
                Some((path, lock))
            });
        let original_paths = project.paths.clone();
        if let Some((path, _)) = &cache {
            project.cached = true;
            project.no_artifacts = false;
            project.paths.cache = path.join(SOLIDITY_FILES_CACHE_FILENAME);
            project.paths.artifacts = path.join("artifacts");
            project.paths.build_infos = path.join("build-info");
            project.paths.slash_paths();
        }

        if ir_minimum {
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

        config.disable_optimizations(&mut project, ir_minimum);

        let files = (filter.args().path_pattern.is_some()
            || filter.args().path_pattern_inverse.is_some())
        .then(|| {
            source_files_iter(&config.src, MultiCompilerLanguage::FILE_EXTENSIONS)
                .chain(
                    source_files_iter(&config.test, MultiCompilerLanguage::FILE_EXTENSIONS)
                        // Preserve path-filter behavior for conventional test files while still
                        // scanning non-test fixtures under the test root.
                        .filter(|path| !path.is_sol_test() || filter.matches_path(path)),
                )
                // Coverage reports include scripts even though they are not test targets.
                .chain(source_files_iter(&config.script, MultiCompilerLanguage::FILE_EXTENSIONS))
                .collect::<BTreeSet<_>>()
        });
        let compile = |project: &Project| {
            let mut compiler = ProjectCompiler::new()
                .allow_linked_libraries(config.allow_linked_libraries)
                .external_compilers(config)
                .external_artifacts(false)
                .dynamic_test_linking(config.dynamic_test_linking);
            if let Some(files) = &files {
                compiler = compiler.files(files.iter().cloned());
            }
            compiler.compile(project)
        };
        // Coverage needs complete build contexts, including sources without contract artifacts.
        // Invalidate the owned cache as a whole on edits so source IDs, artifact names, and
        // build-info files all belong to the same compilation.
        let cached_output = (|| -> Result<Option<ProjectCompileOutput>> {
            let Some((path, _)) = &cache else { return Ok(None) };
            let mut sources = if let Some(files) = &files
                && !files.is_empty()
            {
                Source::read_all(files)?
            } else {
                project.paths.read_input_files()?
            };
            if let Some(filter) = &project.sparse_output {
                sources.retain(|path, _| filter.is_match(path));
            }
            let graph = Graph::<MultiCompilerParser>::resolve_sources(&project.paths, sources)?;
            let resolved = graph.into_sources_by_version(&project)?;
            let mut jobs = resolved
                .sources
                .iter()
                .flat_map(|(language, jobs)| {
                    jobs.iter().map(move |(version, sources, (profile, settings))| {
                        let sources = sources
                            .iter()
                            .map(|(path, source)| {
                                (path, source.content_hash(), source.kind.is_dirty())
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value((language, version, profile, settings, sources))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            jobs.sort_unstable_by_key(serde_json::Value::to_string);
            let primary_profiles = resolved.primary_profiles.iter().collect::<BTreeMap<_, _>>();
            let mut identity = serde_json::to_value((
                SHORT_VERSION,
                &project.paths,
                (&config.solc, &config.vyper, &config.extra_args),
                (&config.extra_output, &config.extra_output_files),
                config.dynamic_test_linking,
                jobs,
                primary_profiles,
            ))?;
            identity.sort_all_objects();
            let fingerprint = keccak256(serde_json::to_vec(&identity)?).to_string();
            let marker = path.join(Config::COVERAGE_CACHE_MARKER);
            if fs::read_to_string(&marker).ok().as_deref() != Some(&fingerprint) {
                // Retain ownership even if compilation fails before the new fingerprint is saved.
                fs::write(&marker, "")?;
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    if entry.file_name() == Config::COVERAGE_CACHE_MARKER {
                        continue;
                    }
                    if entry.file_type()?.is_dir() {
                        fs::remove_dir_all(entry.path())?;
                    } else {
                        fs::remove_file(entry.path())?;
                    }
                }
            }
            let output = compile(&project)?;
            // Compiler builds with no artifacts can be pruned from the cache. Coverage still
            // reports their free functions, so recover their source IDs with a fresh compilation.
            let mapped_sources = output
                .builds()
                .flat_map(|(_, build)| build.source_id_to_path.values().map(PathBuf::as_path))
                .collect::<HashSet<_>>();
            if output
                .graph()
                .files()
                .any(|idx| !mapped_sources.contains(output.graph().node_path(idx)))
            {
                return Ok(None);
            }
            // Publication is optional once the compiler has returned a complete output.
            if let Err(err) = fs::write(marker, fingerprint) {
                debug!(%err, "failed to publish coverage cache fingerprint");
            }
            Ok(Some(output))
        })();
        let output = match cached_output {
            Ok(Some(output)) => output,
            Err(err)
                if !err.chain().any(|cause| {
                    cause.is::<std::io::Error>()
                        || matches!(cause.downcast_ref(), Some(SolcError::Io(_)))
                }) =>
            {
                return Err(err);
            }
            result => {
                if let Err(err) = result {
                    debug!(%err, "coverage cache unavailable; compiling without persistence");
                }
                project.cached = false;
                project.no_artifacts = true;
                project.paths = original_paths;
                compile(&project)?
            }
        };
        let output = output.with_stripped_file_prefixes(project.root());

        Ok((project, output))
    }

    /// Builds the coverage report.
    #[instrument(name = "Coverage::prepare", skip_all)]
    fn prepare(
        settings: &CoverageSettings,
        reporters: &[Box<dyn CoverageReporter>],
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
        let mut sources_by_build = HashMap::<String, SourceFiles>::default();
        for (build_id, build) in output.builds() {
            for (source_id, path) in &build.source_id_to_path {
                if output.graph().get_parsed_source(path).is_none()
                    || path
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|ext| VYPER_EXTENSIONS.contains(&ext))
                {
                    continue;
                }
                let path = path.strip_prefix(&project_paths.root).unwrap_or(path);
                report.add_source(build_id.clone(), *source_id as usize, path.to_path_buf());

                if (!settings.include_libs && project_paths.has_library_ancestor(path))
                    || (settings.exclude_tests && project_paths.is_test(path))
                {
                    continue;
                }

                sources_by_build
                    .entry(build_id.clone())
                    .or_default()
                    .sources
                    .insert(*source_id, project_paths.root.join(path));
            }
        }

        // Get source maps and bytecodes.
        let artifacts: Vec<ArtifactData> = output
            .artifact_ids()
            .par_bridge() // This parses source maps, so we want to run it in parallel.
            .filter_map(|(id, artifact)| {
                let source_id = report.get_source_id(&id.build_id, &id.source)?;
                ArtifactData::new(&id, source_id, artifact)
            })
            .collect();

        // Add coverage items.
        for (build_id, sources) in &sources_by_build {
            let source_analysis = SourceAnalysis::new(sources, output)?;
            let anchors = artifacts
                .par_iter()
                .filter(|artifact| artifact.contract_id.build_id == *build_id)
                .map(|artifact| {
                    let creation_code_anchors = artifact.creation.find_anchors(&source_analysis);
                    let deployed_code_anchors = artifact.deployed.find_anchors(&source_analysis);
                    (artifact.contract_id.clone(), (creation_code_anchors, deployed_code_anchors))
                })
                .collect_vec_list();
            report.add_anchors(anchors.into_iter().flatten());
            for artifact in
                artifacts.iter().filter(|artifact| artifact.contract_id.build_id == *build_id)
            {
                let execution_anchors = find_execution_anchors(
                    artifact.contract_id.source_id as u32,
                    &artifact.contract_id.contract_name,
                    &source_analysis,
                );
                report.add_execution_anchors(
                    artifact.contract_id.clone(),
                    execution_anchors,
                    artifact.function_selectors.iter().copied(),
                    artifact.has_receive,
                    artifact.fallback_payable,
                );
            }
            report.add_analysis(build_id.clone(), source_analysis);
        }

        if reporters.iter().any(|reporter| reporter.needs_source_maps()) {
            report.add_source_maps(artifacts.into_iter().map(|artifact| {
                (artifact.contract_id, (artifact.creation.source_map, artifact.deployed.source_map))
            }));
        }

        Ok(report)
    }

    /// Runs tests, collects coverage data and generates the final report.
    #[instrument(name = "Coverage::collect", skip_all)]
    #[allow(clippy::too_many_arguments)]
    async fn collect(
        mut self,
        settings: &CoverageSettings,
        reporters: &mut [Box<dyn CoverageReporter>],
        project_root: &Path,
        output: &ProjectCompileOutput,
        mut report: CoverageReport,
        config: Config,
        evm_opts: EvmOpts,
        filter: ProjectPathsAwareFilter,
    ) -> Result<()> {
        let inline_config = Arc::new(InlineConfig::new_parsed(output, &config)?);
        let outcome = self
            .test
            .run_tests(
                project_root,
                config,
                evm_opts,
                output,
                &filter,
                TestExecutionOptions::coverage(inline_config),
            )
            .await?;

        let known_contracts = outcome.known_contracts.as_ref().unwrap();
        // Linking removes library references. Match runtime against the compiler artifacts to
        // retain those offsets, while keeping linked bytecode for creation matching.
        let runtime_contracts = ContractsByArtifactBuilder::new(
            output.artifact_ids().map(|(id, artifact)| (id, artifact.into())),
        )
        .build();
        let mut resolved_hit_maps = ResolvedHitMaps::default();
        let mut unresolved_hit_maps = HashSet::<B256>::default();

        // Add hit data to the coverage report
        for suite in outcome.results.values() {
            for result in suite.test_results.values() {
                let Some(hit_maps) = result.line_coverage.as_ref() else { continue };

                for (code_hash, map) in &hit_maps.0 {
                    if let Some(resolved) = resolved_hit_maps.get(code_hash) {
                        report.add_hit_map(
                            &resolved.contract_id,
                            map,
                            resolved.is_deployed_code,
                        )?;
                        continue;
                    }
                    if unresolved_hit_maps.contains(code_hash) {
                        continue;
                    }

                    // Only exact matches are credited: source maps of a merely similar artifact
                    // would attribute hits to code that never ran. Creation code may be followed
                    // by any constructor data, which leaves the artifact's instructions unchanged.
                    let Some((artifact_id, is_deployed_code)) = runtime_contracts
                        .find_by_deployed_code_exact(map.bytecode())
                        .map(|(id, _)| (id, true))
                        .or_else(|| {
                            known_contracts
                                .iter()
                                .find(|(_, contract)| {
                                    contract
                                        .bytecode()
                                        .is_some_and(|code| map.bytecode().starts_with(code))
                                })
                                .map(|(id, _)| (id, false))
                        })
                    else {
                        if let Some((similar, kind)) = known_contracts
                            .find_by_deployed_code(map.bytecode())
                            .map(|(id, _)| (id, "runtime"))
                            .or_else(|| {
                                known_contracts
                                    .find_by_creation_code(map.bytecode())
                                    .map(|(id, _)| (id, "creation"))
                            })
                        {
                            sh_warn!(
                                "excluding executed {kind} code from coverage: it resembles `{}` but does not match it exactly",
                                similar.identifier()
                            )?;
                        }
                        unresolved_hit_maps.insert(*code_hash);
                        continue;
                    };

                    let Some(source_id) =
                        report.get_source_id(&artifact_id.build_id, &artifact_id.source)
                    else {
                        continue;
                    };
                    let contract_id = ContractId {
                        version: artifact_id.version.clone(),
                        build_id: artifact_id.build_id.clone(),
                        source_id,
                        contract_name: artifact_id.name.as_str().into(),
                    };

                    report.add_hit_map(&contract_id, map, is_deployed_code)?;

                    resolved_hit_maps
                        .entry(*code_hash)
                        .or_insert(ResolvedHitMap { contract_id, is_deployed_code });
                }
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
        if !settings.skip_files.is_empty() {
            let mut builder = GlobSetBuilder::new();
            for pattern in &settings.skip_files {
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
        Self::report(reporters, &report)?;

        if settings.report.iter().any(|kind| matches!(kind, CoverageReportKind::Attribution)) {
            let reporter = CoverageAttributionReporter::new(
                settings.report_path(project_root, "coverage-attribution.json"),
            );
            reporter.report(&report, &outcome, &resolved_hit_maps)?;
        }

        // Check for test failures after generating coverage report.
        // This ensures coverage data is written even when tests fail.
        outcome.ensure_ok(false)?;

        Ok(())
    }

    #[instrument(name = "Coverage::report", skip_all)]
    fn report(reporters: &mut [Box<dyn CoverageReporter>], report: &CoverageReport) -> Result<()> {
        for reporter in reporters {
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
    pub function_selectors: Vec<[u8; 4]>,
    pub has_receive: bool,
    pub fallback_payable: bool,
}

impl ArtifactData {
    pub fn new(id: &ArtifactId, source_id: usize, artifact: &impl Artifact) -> Option<Self> {
        let abi = artifact.get_abi();
        let function_selectors = abi
            .as_ref()
            .map(|abi| abi.functions().map(|function| function.selector().into()).collect())
            .unwrap_or_default();
        let has_receive = abi.as_ref().is_some_and(|abi| abi.receive.is_some());
        let fallback_payable = abi
            .as_ref()
            .and_then(|abi| abi.fallback)
            .is_some_and(|fallback| fallback.state_mutability == StateMutability::Payable);
        Some(Self {
            contract_id: ContractId {
                version: id.version.clone(),
                build_id: id.build_id.clone(),
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
            function_selectors,
            has_receive,
            fallback_payable,
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

    pub fn find_anchors(&self, source_analysis: &SourceAnalysis) -> ItemAnchors {
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
        let args = CoverageArgs::parse_from(["coverage"]);
        let config = CoverageConfig { lcov_version: Version::new(2, 2, 0), ..Default::default() };

        let settings = args.resolve(&config);

        assert_eq!(settings.lcov_version, Version::new(2, 2, 0));
    }

    #[test]
    fn resolve_lcov_version_keeps_explicit_cli_default() {
        let args = CoverageArgs::parse_from(["coverage", "--lcov-version", "1"]);
        let config = CoverageConfig { lcov_version: Version::new(2, 2, 0), ..Default::default() };

        let settings = args.resolve(&config);

        assert_eq!(settings.lcov_version, Version::new(1, 0, 0));
    }

    #[test]
    fn resolve_lcov_version_keeps_explicit_cli_value() {
        let args = CoverageArgs::parse_from(["coverage", "--lcov-version", "2"]);
        let config = CoverageConfig { lcov_version: Version::new(2, 2, 0), ..Default::default() };

        let settings = args.resolve(&config);

        assert_eq!(settings.lcov_version, Version::new(2, 0, 0));
    }
}
