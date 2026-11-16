//! Support for compiling [foundry_compilers::Project]

use crate::{
    TestFunctionExt,
    external_compiler::{ExternalCompilation, is_builtin_compiler_source},
    fs::canonicalize_path,
    preprocessor::DynamicTestLinkingPreprocessor,
    shell,
    term::SpinnerReporter,
};
use alloy_json_abi::JsonAbi;
use comfy_table::{
    Cell, Color, Table,
    presets::{ASCII_FULL, ASCII_MARKDOWN},
};
use evm2::{
    SpecId,
    constants::{
        MAX_CODE_SIZE, MAX_CODE_SIZE_AMSTERDAM, MAX_INITCODE_SIZE, MAX_INITCODE_SIZE_AMSTERDAM,
    },
};
use eyre::{OptionExt, Result};
use foundry_block_explorers::{contract::Metadata, source_tree::SourceTree};
use foundry_compilers::{
    Artifact, Project, ProjectBuilder, ProjectCompileOutput, ProjectPathsConfig, SolcConfig,
    artifacts::{
        BytecodeObject, Contract, Error as SolcError, SecondarySourceLocation, Severity, Source,
        output_selection::OutputSelection,
        remappings::{RelativeRemapping, Remapping},
    },
    compilers::{
        CompilationError, Compiler,
        solc::{Solc, SolcCompiler},
    },
    info::ContractInfo as CompilerContractInfo,
    multi::{MultiCompiler, MultiCompilerError, MultiCompilerSettings},
    project::Preprocessor,
    report::{BasicStdoutReporter, NoReporter, Report},
    solc::SolcSettings,
};
use foundry_config::Config;
use num_format::{Locale, ToFormattedString};
use solar::{
    ast::{Arena, ContractKind, ItemKind},
    interface::{Session, source_map::FileName},
    parse::Parser,
};
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt::Display,
    io::IsTerminal,
    path::{Component, Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Instant,
};

/// A Solar compiler instance, to grant syntactic and semantic analysis capabilities.
pub type Analysis = Arc<solar::sema::Compiler>;

/// Builder type to configure how to compile a project.
///
/// This is merely a wrapper for [`Project::compile()`] which also prints to stdout depending on its
/// settings.
#[must_use = "ProjectCompiler does nothing unless you call a `compile*` method"]
pub struct ProjectCompiler {
    /// The root of the project.
    project_root: PathBuf,

    /// Whether to also print contract names.
    print_names: Option<bool>,

    /// Whether to also print contract sizes.
    print_sizes: Option<bool>,

    /// Whether to print anything at all. Overrides other `print` options.
    quiet: Option<bool>,

    /// Whether to print the resolved settings for each compiler invocation.
    print_compiler_settings: bool,

    /// Whether to bail on compiler errors.
    bail: Option<bool>,

    /// Whether to ignore the contract initcode size limit introduced by EIP-3860.
    ignore_eip_3860: bool,

    /// Contract size limits used when reporting compiled contract sizes.
    size_limits: ContractSizeLimits,

    /// Extra files to include, that are not necessarily in the project's source directory.
    files: Vec<PathBuf>,

    /// Complete source roots to retry if filtering changed Solidity's inheritance ordering.
    source_order_fallback: Vec<PathBuf>,

    /// Paths used by external adapters to select compiler-native build units.
    selected_paths: Vec<PathBuf>,

    /// Whether to compile with dynamic linking tests and scripts.
    dynamic_test_linking: bool,

    /// Whether compiled contracts may depend on externally linked Solidity libraries.
    allow_linked_libraries: bool,

    /// Whether ABI acquisition may consult the compiler-owned ABI cache.
    abi_cache: bool,

    /// External compiler configuration.
    external_compilers: Option<Config>,

    /// Preserves the caller's artifact policy when ABI caching enables artifacts on a cloned
    /// project.
    external_writes: bool,
}

impl Default for ProjectCompiler {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectCompiler {
    /// Create a new builder with the default settings.
    #[inline]
    pub fn new() -> Self {
        Self {
            project_root: PathBuf::new(),
            print_names: None,
            print_sizes: None,
            quiet: Some(crate::shell::is_quiet()),
            print_compiler_settings: false,
            bail: None,
            ignore_eip_3860: false,
            size_limits: ContractSizeLimits::default(),
            files: Vec::new(),
            source_order_fallback: Vec::new(),
            selected_paths: Vec::new(),
            dynamic_test_linking: false,
            allow_linked_libraries: true,
            abi_cache: false,
            external_compilers: None,
            external_writes: true,
        }
    }

    /// Sets whether to print contract names.
    #[inline]
    pub const fn print_names(mut self, yes: bool) -> Self {
        self.print_names = Some(yes);
        self
    }

    /// Sets whether to print contract sizes.
    #[inline]
    pub const fn print_sizes(mut self, yes: bool) -> Self {
        self.print_sizes = Some(yes);
        self
    }

    /// Sets whether to print anything at all. Overrides other `print` options.
    #[inline]
    #[doc(alias = "silent")]
    pub const fn quiet(mut self, yes: bool) -> Self {
        self.quiet = Some(yes);
        self
    }

    /// Sets whether to print the resolved settings for each compiler invocation.
    #[inline]
    pub const fn print_compiler_settings(mut self, yes: bool) -> Self {
        self.print_compiler_settings = yes;
        self
    }

    /// Sets whether to bail on compiler errors.
    #[inline]
    pub const fn bail(mut self, yes: bool) -> Self {
        self.bail = Some(yes);
        self
    }

    /// Sets whether to ignore EIP-3860 initcode size limits.
    #[inline]
    pub const fn ignore_eip_3860(mut self, yes: bool) -> Self {
        self.ignore_eip_3860 = yes;
        self
    }

    /// Sets the contract size limits for size reports.
    #[inline]
    pub const fn size_limits(mut self, limits: ContractSizeLimits) -> Self {
        self.size_limits = limits;
        self
    }

    /// Sets extra files to include, that are not necessarily in the project's source dir.
    #[inline]
    pub fn files(mut self, files: impl IntoIterator<Item = PathBuf>) -> Self {
        self.files.extend(files);
        self
    }

    /// Sets paths used by external adapters to select compiler-native build units.
    pub fn selected_paths(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.selected_paths.extend(paths);
        self
    }

    /// Selects targets for external adapters and built-in sources where applicable.
    pub fn target_files(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        for path in paths {
            if is_builtin_compiler_source(&path) {
                self.files.push(path.clone());
            }
            self.selected_paths.push(path);
        }
        self
    }

    /// Sets if tests should be dynamically linked.
    #[inline]
    pub const fn dynamic_test_linking(mut self, preprocess: bool) -> Self {
        self.dynamic_test_linking = preprocess;
        self
    }

    /// Enables explicitly configured external compilers for this build.
    pub fn external_compilers(mut self, config: &Config) -> Self {
        self.external_compilers = Some(config.clone());
        self
    }

    /// Controls whether external artifacts and their cache may be published.
    pub const fn external_artifacts(mut self, write: bool) -> Self {
        self.external_writes = write;
        self
    }

    /// Reuses normal artifacts before consulting the compiler-owned ABI cache.
    ///
    /// The project must request ABI output and may include other contract outputs.
    /// The project's artifact policy controls writes to the secondary cache.
    pub const fn cache_abi(mut self) -> Self {
        self.abi_cache = true;
        self
    }

    /// Retries once with these roots if compilation fails because a base contract is defined
    /// after its derived contract. Removing roots can change traversal order in import cycles.
    pub fn source_order_fallback(mut self, files: impl IntoIterator<Item = PathBuf>) -> Self {
        self.source_order_fallback.extend(files);
        self
    }

    /// Sets whether compiled contracts may depend on externally linked Solidity libraries.
    ///
    /// This does not disable dynamic test linking. With `bail(false)`, violations are reported as
    /// compiler errors in the returned output.
    pub const fn allow_linked_libraries(mut self, allow: bool) -> Self {
        self.allow_linked_libraries = allow;
        self
    }

    /// Compiles the project.
    #[instrument(target = "forge::compile", skip_all)]
    pub fn compile<C: Compiler<CompilerContract = Contract>>(
        mut self,
        project: &Project<C>,
    ) -> Result<ProjectCompileOutput<C>>
    where
        DynamicTestLinkingPreprocessor: Preprocessor<C>,
    {
        self.project_root = project.root().to_path_buf();

        // TODO: Avoid using std::process::exit(0).
        // Replacing this with a return (e.g., Ok(ProjectCompileOutput::default())) would be more
        // idiomatic, but it currently requires a `Default` bound on `C::Language`, which
        // breaks compatibility with downstream crates like `foundry-cli`. This would need a
        // broader refactor across the call chain. Leaving it as-is for now until a larger
        // refactor is feasible.
        if !project.paths.has_input_files()
            && self.files.is_empty()
            && self.selected_paths.is_empty()
            && self
                .external_compilers
                .as_ref()
                .is_none_or(|config| config.external_compilers.is_empty())
        {
            sh_println!("Nothing to compile")?;
            std::process::exit(0);
        }

        // Taking is fine since we don't need these in `compile_with`.
        let files = std::mem::take(&mut self.files);
        let fallback_files = std::mem::take(&mut self.source_order_fallback);
        let explicit_selection = !files.is_empty() || !self.selected_paths.is_empty();
        let selected_paths = std::mem::take(&mut self.selected_paths);
        let preprocess = self.dynamic_test_linking;
        let abi_cache = self.abi_cache;
        let external_compilers = self.external_compilers.take();
        let external_writes = self.external_writes && !project.no_artifacts;
        self.compile_with(|| {
            let external = external_compilers
                .as_ref()
                .map(|config| {
                    ExternalCompilation::compile(config, &selected_paths, external_writes)
                })
                .transpose()?;
            let sources = if explicit_selection {
                Source::read_all(files)?
            } else {
                project.paths.read_input_files()?
            };

            let preprocessor = DynamicTestLinkingPreprocessor::default();
            let compile = |preprocess| -> Result<ProjectCompileOutput<C>> {
                let compile_sources = |sources| {
                    let mut compiler = foundry_compilers::project::ProjectCompiler::with_sources(
                        project, sources,
                    )?;
                    if preprocess {
                        compiler = compiler.with_preprocessor(preprocessor.clone());
                    }
                    if abi_cache { compiler.compile_abi_cached() } else { compiler.compile() }
                };
                let mut output = compile_sources(sources.clone())?;
                // Solc error 2449 can be introduced by filtering compilation roots in an import
                // cycle. Retry before reporting diagnostics or bailing on the filtered output.
                if !fallback_files.is_empty()
                    && output.output().errors.iter().any(|error| error.error_code() == Some(2449))
                {
                    output = compile_sources(Source::read_all(&fallback_files)?)?;
                }
                Ok(output)
            };
            let mut output = compile(preprocess)?;
            // A retry can only help if DTL changed the compiler input.
            if preprocessor.rewrote_sources()
                && let Some(reason) = dynamic_test_linking_failure(&output.output().errors)
            {
                sh_warn!(
                    "dynamic test linking failed ({reason}); retrying with native test linking. \
                     Use --no-dynamic-test-linking to skip the failed attempt."
                )?;
                // The preprocessor is part of the cache identity: native compilation cannot
                // reuse DTL artifacts. Return only this attempt, including any compiler errors.
                output = compile(false)?;
            }
            // Drop deferred DTL-disable warnings on failure; solc's error already explains it.
            let disable_warnings = preprocessor.take_disable_warnings();
            if !output.has_compiler_errors() {
                for warning in disable_warnings {
                    sh_warn!("{warning}")?;
                }
            }
            if !output.has_compiler_errors()
                && let Some(external) = external
            {
                external.merge(&mut output)?;
            }
            add_root_remapping_diagnostics(&mut output, &project.paths);
            Ok(output)
        })
    }

    /// Compiles the project with the given closure
    fn compile_with<C: Compiler<CompilerContract = Contract>, F>(
        self,
        f: F,
    ) -> Result<ProjectCompileOutput<C>>
    where
        F: FnOnce() -> Result<ProjectCompileOutput<C>>,
    {
        let quiet = self.quiet.unwrap_or(false);
        let bail = self.bail.unwrap_or(true);

        let mut output = with_compilation_reporter_and_settings(
            quiet,
            Some(self.project_root.clone()),
            self.print_compiler_settings,
            || {
                tracing::debug!("compiling project");

                let timer = Instant::now();
                let r = f();
                let elapsed = timer.elapsed();

                tracing::debug!("finished compiling in {:.3}s", elapsed.as_secs_f64());
                r
            },
        )?;

        if bail && output.has_compiler_errors() {
            eyre::bail!("{output}");
        }

        let mut linking_error = (!self.allow_linked_libraries && !output.has_compiler_errors())
            .then(|| linked_libraries_error(&output, &self.project_root))
            .flatten();
        if !bail && let Some(message) = linking_error.take() {
            push_linked_libraries_error(&mut output, message);
        }

        if !quiet && !shell::is_json() {
            if output.is_unchanged() {
                sh_println!("No files changed, compilation skipped")?;
            } else {
                // print the compiler output / warnings
                sh_println!("{output}")?;
            }
        }

        // Report compiler warnings before the policy failure.
        if let Some(message) = linking_error {
            eyre::bail!("{message}");
        }

        // Quiet mode suppresses reports, but size limits still apply.
        if !(shell::is_json() && output.has_compiler_errors()) {
            self.handle_output(&output)?;
        }

        Ok(output)
    }

    /// Prints requested reports and checks contract size limits.
    fn handle_output<C: Compiler<CompilerContract = Contract>>(
        &self,
        output: &ProjectCompileOutput<C>,
    ) -> Result<()> {
        let quiet = self.quiet.unwrap_or(false);
        let print_names = self.print_names.unwrap_or(false) && !quiet;
        let print_sizes = self.print_sizes.unwrap_or(false);

        // print any sizes or names
        if print_names {
            let mut artifacts: BTreeMap<_, Vec<_>> = BTreeMap::new();
            for (name, (_, version)) in output.versioned_artifacts() {
                artifacts.entry(version).or_default().push(name);
            }

            if shell::is_json() {
                sh_println!("{}", serde_json::to_string(&artifacts).unwrap())?;
            } else {
                for (version, names) in artifacts {
                    sh_println!(
                        "  compiler version: {}.{}.{}",
                        version.major,
                        version.minor,
                        version.patch
                    )?;
                    for name in names {
                        sh_println!("    - {name}")?;
                    }
                }
            }
        }

        if print_sizes {
            // add extra newline if names were already printed
            if print_names && !shell::is_json() {
                sh_println!()?;
            }

            let mut size_report =
                SizeReport { contracts: BTreeMap::new(), limits: self.size_limits };

            let mut artifacts: BTreeMap<String, Vec<_>> = BTreeMap::new();
            for (id, artifact) in output.artifact_ids().filter(|(id, _)| {
                // filter out forge-std specific contracts
                !id.source.to_string_lossy().contains("/forge-std/src/")
            }) {
                artifacts.entry(id.name.clone()).or_default().push((id.source, artifact));
            }

            // Internal libraries are inlined into consumers and never deployed; skip them.
            // Only artifacts whose ABI has no functions can be internal libraries, so restrict the
            // solar parse to those sources to avoid a second full parse pass.
            let abs_source = |path: &Path| -> PathBuf {
                if path.is_absolute() { path.to_path_buf() } else { self.project_root.join(path) }
            };
            let source_paths = artifacts
                .values()
                .flatten()
                .filter(|(_, artifact)| {
                    artifact.abi.as_ref().is_some_and(|abi| abi.functions().next().is_none())
                })
                .map(|(path, _)| abs_source(path))
                .collect::<BTreeSet<_>>();
            let libraries = collect_libraries(&source_paths);

            for (name, artifact_list) in artifacts {
                // A library with no functions in its ABI is internal-only; fail open if the ABI is
                // missing. Filter first so the duplicate-name suffix below reflects kept contracts.
                let kept = artifact_list
                    .iter()
                    .filter(|(path, artifact)| {
                        let is_library = libraries
                            .get(&abs_source(path))
                            .is_some_and(|libs| libs.contains(&name));
                        let has_no_abi_functions = artifact
                            .abi
                            .as_ref()
                            .is_some_and(|abi| abi.functions().next().is_none());
                        !(is_library && has_no_abi_functions)
                    })
                    .collect::<Vec<_>>();

                for (path, artifact) in &kept {
                    let runtime_size = contract_size(*artifact, false).unwrap_or_default();
                    let init_size = contract_size(*artifact, true).unwrap_or_default();

                    let is_dev_contract = artifact.abi.as_ref().is_some_and(|abi| {
                        abi.functions().any(|f| {
                            f.test_function_kind().is_known()
                                || matches!(f.name.as_str(), "IS_TEST" | "IS_SCRIPT")
                        })
                    });

                    let unique_name = if kept.len() > 1 {
                        format!(
                            "{} ({})",
                            name,
                            path.strip_prefix(&self.project_root).unwrap_or(path).display()
                        )
                    } else {
                        name.clone()
                    };

                    size_report.contracts.insert(
                        unique_name,
                        ContractInfo { runtime_size, init_size, is_dev_contract },
                    );
                }
            }

            if !quiet {
                sh_println!("{size_report}")?;
            }

            let runtime_eip = match size_report.limits.runtime {
                CONTRACT_RUNTIME_SIZE_LIMIT => "EIP-170: ",
                AMSTERDAM_CONTRACT_RUNTIME_SIZE_LIMIT => "EIP-7954: ",
                _ => "",
            };
            eyre::ensure!(
                !size_report.exceeds_runtime_size_limit(),
                "some contracts exceed the runtime size limit ({runtime_eip}{} bytes)",
                size_report.limits.runtime
            );
            // Check size limits only if not ignoring EIP-3860
            let initcode_eip = match size_report.limits.initcode {
                CONTRACT_INITCODE_SIZE_LIMIT => "EIP-3860: ",
                AMSTERDAM_CONTRACT_INITCODE_SIZE_LIMIT => "EIP-7954: ",
                _ => "",
            };
            eyre::ensure!(
                self.ignore_eip_3860 || !size_report.exceeds_initcode_size_limit(),
                "some contracts exceed the initcode size limit ({initcode_eip}{} bytes)",
                size_report.limits.initcode
            );
        }

        Ok(())
    }
}

// https://eips.ethereum.org/EIPS/eip-170
const CONTRACT_RUNTIME_SIZE_LIMIT: usize = MAX_CODE_SIZE;

// https://eips.ethereum.org/EIPS/eip-3860
const CONTRACT_INITCODE_SIZE_LIMIT: usize = MAX_INITCODE_SIZE;

// https://eips.ethereum.org/EIPS/eip-7954
const AMSTERDAM_CONTRACT_RUNTIME_SIZE_LIMIT: usize = MAX_CODE_SIZE_AMSTERDAM;
const AMSTERDAM_CONTRACT_INITCODE_SIZE_LIMIT: usize = MAX_INITCODE_SIZE_AMSTERDAM;

const CONTRACT_RUNTIME_SIZE_WARN_THRESHOLD: usize = 18_000;
const CONTRACT_INITCODE_SIZE_WARN_THRESHOLD: usize = 36_000;

/// Runtime and initcode byte-size limits for compiled contract size reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContractSizeLimits {
    /// Maximum deployed runtime bytecode size.
    pub runtime: usize,
    /// Maximum initcode bytecode size.
    pub initcode: usize,
}

impl ContractSizeLimits {
    /// Creates a new set of contract size limits.
    pub const fn new(runtime: usize, initcode: usize) -> Self {
        Self { runtime, initcode }
    }

    /// Creates limits from a runtime code-size limit, using the EIP-3860 2x initcode ratio.
    pub const fn with_runtime_limit(runtime: usize) -> Self {
        Self { runtime, initcode: runtime.saturating_mul(2) }
    }

    /// Returns the protocol limits active for an EVM specification.
    pub const fn for_spec_id(spec_id: SpecId) -> Self {
        if spec_id.enables(SpecId::AMSTERDAM) {
            Self::new(AMSTERDAM_CONTRACT_RUNTIME_SIZE_LIMIT, AMSTERDAM_CONTRACT_INITCODE_SIZE_LIMIT)
        } else {
            Self::new(CONTRACT_RUNTIME_SIZE_LIMIT, CONTRACT_INITCODE_SIZE_LIMIT)
        }
    }

    const fn runtime_warning_threshold(self) -> usize {
        scaled_threshold(
            self.runtime,
            CONTRACT_RUNTIME_SIZE_WARN_THRESHOLD,
            CONTRACT_RUNTIME_SIZE_LIMIT,
        )
    }

    const fn initcode_warning_threshold(self) -> usize {
        scaled_threshold(
            self.initcode,
            CONTRACT_INITCODE_SIZE_WARN_THRESHOLD,
            CONTRACT_INITCODE_SIZE_LIMIT,
        )
    }
}

impl Default for ContractSizeLimits {
    fn default() -> Self {
        Self::new(CONTRACT_RUNTIME_SIZE_LIMIT, CONTRACT_INITCODE_SIZE_LIMIT)
    }
}

const fn scaled_threshold(limit: usize, threshold: usize, default_limit: usize) -> usize {
    limit.saturating_mul(threshold) / default_limit
}

/// Contracts with info about their size
pub struct SizeReport {
    /// `contract name -> info`
    pub contracts: BTreeMap<String, ContractInfo>,
    /// Size limits used to calculate margins and failures.
    pub limits: ContractSizeLimits,
}

impl SizeReport {
    /// Returns the maximum runtime code size, excluding dev contracts.
    pub fn max_runtime_size(&self) -> usize {
        self.contracts
            .values()
            .filter(|c| !c.is_dev_contract)
            .map(|c| c.runtime_size)
            .max()
            .unwrap_or(0)
    }

    /// Returns the maximum initcode size, excluding dev contracts.
    pub fn max_init_size(&self) -> usize {
        self.contracts
            .values()
            .filter(|c| !c.is_dev_contract)
            .map(|c| c.init_size)
            .max()
            .unwrap_or(0)
    }

    /// Returns true if any contract exceeds the runtime size limit, excluding dev contracts.
    pub fn exceeds_runtime_size_limit(&self) -> bool {
        self.max_runtime_size() > self.limits.runtime
    }

    /// Returns true if any contract exceeds the initcode size limit, excluding dev contracts.
    pub fn exceeds_initcode_size_limit(&self) -> bool {
        self.max_init_size() > self.limits.initcode
    }
}

impl Display for SizeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        if shell::is_json() {
            writeln!(f, "{}", self.format_json_output())?;
        } else {
            writeln!(f, "\n{}", self.format_table_output())?;
        }
        Ok(())
    }
}

impl SizeReport {
    fn format_json_output(&self) -> String {
        let contracts = self
            .contracts
            .iter()
            .filter(|(_, c)| !c.is_dev_contract && (c.runtime_size > 0 || c.init_size > 0))
            .map(|(name, contract)| {
                (
                    name.clone(),
                    serde_json::json!({
                        "runtime_size": contract.runtime_size,
                        "init_size": contract.init_size,
                        "runtime_margin": self.limits.runtime as isize - contract.runtime_size as isize,
                        "init_margin": self.limits.initcode as isize - contract.init_size as isize,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();

        serde_json::to_string(&contracts).unwrap()
    }

    fn format_table_output(&self) -> Table {
        let mut table = Table::new();
        if shell::is_markdown() {
            table.load_style(ASCII_MARKDOWN);
        } else {
            table.load_style(ASCII_FULL.with_rounded_corners());
        }

        table.set_header(vec![
            Cell::new("Contract"),
            Cell::new("Runtime Size (B)"),
            Cell::new("Initcode Size (B)"),
            Cell::new("Runtime Margin (B)"),
            Cell::new("Initcode Margin (B)"),
        ]);

        // Filters out dev contracts (Test or Script)
        let contracts = self
            .contracts
            .iter()
            .filter(|(_, c)| !c.is_dev_contract && (c.runtime_size > 0 || c.init_size > 0));
        let runtime_warning_threshold = self.limits.runtime_warning_threshold();
        let initcode_warning_threshold = self.limits.initcode_warning_threshold();
        for (name, contract) in contracts {
            let runtime_margin = self.limits.runtime as isize - contract.runtime_size as isize;
            let init_margin = self.limits.initcode as isize - contract.init_size as isize;

            let runtime_color = if contract.runtime_size < runtime_warning_threshold {
                Color::Reset
            } else if contract.runtime_size <= self.limits.runtime {
                Color::Yellow
            } else {
                Color::Red
            };

            let init_color = if contract.init_size < initcode_warning_threshold {
                Color::Reset
            } else if contract.init_size <= self.limits.initcode {
                Color::Yellow
            } else {
                Color::Red
            };

            let locale = &Locale::en;
            table.add_row([
                Cell::new(name),
                Cell::new(contract.runtime_size.to_formatted_string(locale)).fg(runtime_color),
                Cell::new(contract.init_size.to_formatted_string(locale)).fg(init_color),
                Cell::new(runtime_margin.to_formatted_string(locale)).fg(runtime_color),
                Cell::new(init_margin.to_formatted_string(locale)).fg(init_color),
            ]);
        }

        table
    }
}

/// Parses each source file with solar and returns the library names declared in it.
///
/// Files that fail to parse are skipped, so a missing entry means "unknown", not "no libraries".
fn collect_libraries(sources: &BTreeSet<PathBuf>) -> HashMap<PathBuf, HashSet<String>> {
    let mut result: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    let sess = Session::builder().with_silent_emitter(None).build();
    let _ = sess.enter(|| -> solar::interface::Result<()> {
        for path in sources {
            let arena = Arena::new();
            let mut parser = match Parser::from_lazy_source_code(
                &sess,
                &arena,
                FileName::from(path.clone()),
                || std::fs::read_to_string(path),
            ) {
                Ok(parser) => parser,
                Err(_) => continue,
            };
            let Ok(ast) = parser.parse_file() else { continue };
            let libs = ast
                .items
                .iter()
                .filter_map(|item| match &item.kind {
                    ItemKind::Contract(c) if c.kind == ContractKind::Library => {
                        Some(c.name.as_str().to_string())
                    }
                    _ => None,
                })
                .collect::<HashSet<_>>();
            if !libs.is_empty() {
                result.insert(path.clone(), libs);
            }
        }
        Ok(())
    });
    result
}

/// Returns the deployed or init size of the contract.
fn contract_size<T: Artifact>(artifact: &T, initcode: bool) -> Option<usize> {
    let bytecode = if initcode {
        artifact.get_bytecode_object()?
    } else {
        artifact.get_deployed_bytecode_object()?
    };

    let size = match bytecode.as_ref() {
        BytecodeObject::Bytecode(bytes) => bytes.len(),
        BytecodeObject::Unlinked(unlinked) => {
            // we don't need to account for placeholders here, because library placeholders take up
            // 40 characters: `__$<library hash>$__` which is the same as a 20byte address in hex.
            let mut size = unlinked.len();
            if unlinked.starts_with("0x") {
                size -= 2;
            }
            // hex -> bytes
            size / 2
        }
    };

    Some(size)
}

/// How big the contract is and whether it is a dev contract where size limits can be neglected
#[derive(Clone, Copy, Debug)]
pub struct ContractInfo {
    /// Size of the runtime code in bytes
    pub runtime_size: usize,
    /// Size of the initcode in bytes
    pub init_size: usize,
    /// A development contract is either a Script or a Test contract.
    pub is_dev_contract: bool,
}

/// Compiles target file path.
///
/// If `quiet` is set, the compilation reporter's progress/status output is suppressed.
/// (When not suppressed, that output is emitted to stderr; see `with_compilation_reporter`.)
///
/// **Note:** this expects the `target_path` to be absolute
pub fn compile_target<C: Compiler<CompilerContract = Contract>>(
    target_path: &Path,
    project: &Project<C>,
    quiet: bool,
) -> Result<ProjectCompileOutput<C>>
where
    DynamicTestLinkingPreprocessor: Preprocessor<C>,
{
    ProjectCompiler::new().quiet(quiet).files([target_path.into()]).compile(project)
}

/// Compiles the project requesting only ABI output.
pub fn compile_abi_project<C: Compiler<CompilerContract = Contract>>(
    project: &mut Project<C>,
    mut compiler: ProjectCompiler,
) -> Result<ProjectCompileOutput<C>>
where
    DynamicTestLinkingPreprocessor: Preprocessor<C>,
{
    project.update_output_selection(|selection| {
        // Request ABI so compilers populate `contracts` without producing bytecode outputs.
        *selection = OutputSelection::common_output_selection(["abi".to_string()]);
    });
    // Cached ABI artifacts do not retain compiler diagnostics.
    compiler.abi_cache = (compiler.abi_cache || project.no_artifacts)
        && project.compiler_severity_filter == Severity::Error;
    compiler.compile(project)
}

/// Acquires ABI output with compiler-owned persistence separate from normal artifacts.
///
/// Requests for additional files or full build info retain their existing output behavior.
pub fn compile_abi_project_cached<C: Compiler<CompilerContract = Contract>>(
    project: &mut Project<C>,
    mut compiler: ProjectCompiler,
) -> Result<ProjectCompileOutput<C>>
where
    DynamicTestLinkingPreprocessor: Preprocessor<C>,
{
    if !project.cached
        || project.build_info
        || project.artifacts.additional_files != Default::default()
        || project.compiler_severity_filter != Severity::Error
    {
        return compile_abi_project(project, compiler);
    }
    let mut cached_project = project.clone();
    cached_project.no_artifacts = false;
    compiler.abi_cache = true;
    compiler.external_writes &= !project.no_artifacts;
    compile_abi_project(&mut cached_project, compiler)
}

/// Compiles the target contract requesting only ABI output and returns its ABI.
pub fn compile_target_abi(
    project: &mut Project<MultiCompiler>,
    target_path: &Path,
    target_name: &str,
) -> Result<JsonAbi> {
    let target_path = canonicalize_path(target_path)?;
    let output = compile_abi_project(
        project,
        ProjectCompiler::new().quiet(true).files([target_path.clone()]),
    )?;

    let artifact = output
        .find(&target_path, target_name)
        .ok_or_eyre("failed to find target artifact when compiling for abi")?;
    artifact.abi.clone().ok_or_eyre("target artifact does not have an ABI")
}

/// Creates a [Project] from an Etherscan source.
pub fn etherscan_project(metadata: &Metadata, target_path: &Path) -> Result<Project> {
    let target_path = canonicalize_path(target_path)?;
    write_etherscan_sources(std::slice::from_ref(metadata), &target_path)?;
    let sources_path = target_path.join(&metadata.contract_name);

    let mut settings = metadata.settings()?;

    // make remappings absolute with our root
    //
    // The remappings come from the explorer's copy of the contract's compiler settings, which is
    // attacker controlled: a target of `../../..` would otherwise point the compiler at sources
    // outside the checkout and let a verified contract pull arbitrary local files into the build.
    for remapping in &mut settings.remappings {
        let new_path = sources_path.join(sanitize_relative_path(remapping.path.as_ref()));
        remapping.path = new_path.display().to_string();
    }

    // add missing remappings
    if !settings.remappings.iter().any(|remapping| remapping.name.starts_with("@openzeppelin/")) {
        let oz = Remapping {
            context: None,
            name: "@openzeppelin/".into(),
            path: sources_path.join("@openzeppelin").display().to_string(),
        };
        settings.remappings.push(oz);
    }

    // root/
    //   ContractName/
    //     [source code]
    let paths = ProjectPathsConfig::builder()
        .sources(sources_path.clone())
        .remappings(settings.remappings.clone())
        .build_with_root(sources_path);

    // TODO: detect vyper
    let v = metadata.compiler_version()?;
    let solc = Solc::find_or_install(&v)?;

    let compiler = MultiCompiler { solc: Some(SolcCompiler::Specific(solc)), vyper: None };

    Ok(ProjectBuilder::<MultiCompiler>::default()
        .settings(MultiCompilerSettings {
            solc: SolcSettings {
                settings: SolcConfig::builder().settings(settings).build(),
                ..Default::default()
            },
            ..Default::default()
        })
        .paths(paths)
        .ephemeral()
        .no_artifacts()
        .build(compiler)?)
}

/// Writes explorer sources after validating every contract name and source path.
///
/// Each source must remain below its own non-hidden contract directory. All names and paths in
/// the reply are validated before any files are written, and existing symlink components are
/// rejected.
pub fn write_etherscan_sources(metadata: &[Metadata], target_path: &Path) -> Result<()> {
    let mut source_tree = SourceTree { entries: Vec::new() };
    for item in metadata {
        ensure_safe_contract_name(&item.contract_name)?;
        // Resolve existing NTFS short names before treating the contract directory as non-hidden.
        #[cfg(windows)]
        match canonicalize_path(target_path.join(&item.contract_name)) {
            Ok(path) => {
                let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
                    eyre::eyre!("invalid contract directory from explorer: {path:?}")
                })?;
                ensure_safe_contract_name(name)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let entries = item.source_entries();
        for entry in &entries {
            eyre::ensure!(
                source_path_is_scoped(&entry.path, &item.contract_name),
                "invalid source path from explorer: {:?}",
                entry.path
            );
        }
        source_tree.entries.extend(entries);
    }

    for path in source_tree_write_paths(&source_tree) {
        ensure_no_symlink_components(target_path, &path)?;
    }

    source_tree.write_to(target_path)?;
    Ok(())
}

/// Ensures that an explorer-supplied contract name is a single, non-hidden directory component.
///
/// The source tree roots every source under `Path::new(contract_name)`, and its entries are written
/// with `std::fs::write`. A name such as `""` or `"."` would therefore place the reply's files
/// directly in the output directory and silently replace existing files there, while `.git` would
/// let it write into repository metadata.
fn ensure_safe_contract_name(contract_name: &str) -> Result<()> {
    let mut components = Path::new(contract_name).components();
    let safe = matches!(
        components.next(),
        Some(Component::Normal(name))
            if name.to_str().is_some_and(is_safe_output_path_component)
    ) && components.next().is_none()
        && !contract_name.starts_with('.')
        && !(cfg!(target_os = "macos") && has_hfs_ignored_prefix_before_dot(contract_name));
    eyre::ensure!(safe, "invalid contract name from explorer: {contract_name:?}");
    Ok(())
}

fn has_hfs_ignored_prefix_before_dot(name: &str) -> bool {
    name.chars().find(|character| !is_hfs_ignored_character(*character)) == Some('.')
}

/// HFS+ ignores these codepoints during catalog comparison, matching Git's `core.protectHFS` list.
const fn is_hfs_ignored_character(character: char) -> bool {
    matches!(
        character as u32,
        0x200c..=0x200f | 0x202a..=0x202e | 0x206a..=0x206f | 0xfeff
    )
}

fn is_safe_output_path_component(component: &str) -> bool {
    !cfg!(windows) || is_safe_windows_path_component(component)
}

/// Reject names that Win32 treats as path syntax, aliases, or device names.
fn is_safe_windows_path_component(component: &str) -> bool {
    if component.is_empty()
        || component.ends_with('.')
        || component.ends_with(' ')
        || component.chars().any(|character| {
            character < ' '
                || matches!(character, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
    {
        return false;
    }

    // NTFS may expose `.git` through this 8.3 alias, as recognized by Git's `core.protectNTFS`.
    if component.eq_ignore_ascii_case("git~1") {
        return false;
    }

    let device_name =
        component.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_uppercase();
    !matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        && !["COM", "LPT"].iter().any(|prefix| {
            device_name.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

fn source_path_is_scoped(source_path: &Path, contract_name: &str) -> bool {
    let Ok(relative_path) = source_path.strip_prefix(contract_name) else { return false };
    let mut components = relative_path.components();
    matches!(
        components.next(),
        Some(Component::Normal(name))
            if name.to_str().is_some_and(is_safe_output_path_component)
    ) && components.all(|component| {
        matches!(
            component,
            Component::Normal(name) if name.to_str().is_some_and(is_safe_output_path_component)
        )
    })
}

/// Mirrors `SourceTree::write_to`'s extension fallback for symlink preflight.
fn source_tree_write_paths(source_tree: &SourceTree) -> impl Iterator<Item = PathBuf> + '_ {
    source_tree.entries.iter().map(|entry| {
        let mut path = entry.path.clone();
        if path.extension().is_none() {
            let with_extension = path.with_extension("sol");
            if !source_tree.entries.iter().any(|other| other.path == with_extension) {
                path = with_extension;
            }
        }
        path
    })
}

fn ensure_no_symlink_components(target_path: &Path, source_path: &Path) -> Result<()> {
    let mut current_path = target_path.to_path_buf();
    for component in source_path.components() {
        let Component::Normal(name) = component else {
            eyre::bail!("invalid source path from explorer: {source_path:?}");
        };
        current_path.push(name);
        match std::fs::symlink_metadata(&current_path) {
            Ok(metadata) => eyre::ensure!(
                !metadata.file_type().is_symlink(),
                "source path from explorer traverses a symlink: {current_path:?}"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Normalizes an untrusted path relative to the source root, discarding leading separators,
/// drive prefixes, and parent components that would escape that root.
fn sanitize_relative_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                normalized.pop();
            }
            _ => {}
        }
    }
    normalized
}

/// Adds `storageLayout` to the compiler output selection for the given project.
pub fn add_storage_layout_output<C: Compiler<CompilerContract = Contract>>(
    project: &mut Project<C>,
) {
    project.artifacts.additional_values.storage_layout = true;
    project.update_output_selection(|selection| {
        for contract_selection in selection.0.values_mut() {
            for selection in contract_selection.values_mut() {
                selection.push("storageLayout".to_string());
            }
        }
    })
}

/// Configures the reporter and runs the given closure.
///
/// In TTY mode, [`SpinnerReporter`] paints the progress to stderr. The non-TTY fallback
/// still writes to stdout via `BasicStdoutReporter`; migrating that path to stderr is
/// part of the per-command stdout migration tracked in `docs/dev/output-channels.md`
/// (it would shift many existing snapshot tests at once).
pub fn with_compilation_reporter<O>(
    quiet: bool,
    project_root: Option<PathBuf>,
    f: impl FnOnce() -> O,
) -> O {
    with_compilation_reporter_and_settings(quiet, project_root, false, f)
}

fn with_compilation_reporter_and_settings<O>(
    quiet: bool,
    project_root: Option<PathBuf>,
    print_compiler_settings: bool,
    f: impl FnOnce() -> O,
) -> O {
    #[expect(clippy::collapsible_else_if)]
    let reporter = if quiet || shell::is_json() {
        Report::new(NoReporter::default())
    } else {
        if std::io::stderr().is_terminal() {
            Report::new(
                SpinnerReporter::spawn(project_root)
                    .with_compiler_settings(print_compiler_settings),
            )
        } else {
            Report::new(
                BasicStdoutReporter::default().with_compiler_settings(print_compiler_settings),
            )
        }
    };

    foundry_compilers::report::with_scoped(&reporter, f)
}

/// Container type for parsing contract identifiers from CLI.
///
/// Passed string can be of the following forms:
/// - `src/Counter.sol` - path to the contract file, in the case where it only contains one contract
/// - `src/Counter.sol:Counter` - path to the contract file and the contract name
/// - `Counter` - contract name only
#[derive(Clone, PartialEq, Eq)]
pub enum PathOrContractInfo {
    /// Non-canonicalized path provided via CLI.
    Path(PathBuf),
    /// Contract info provided via CLI.
    ContractInfo(CompilerContractInfo),
}

impl PathOrContractInfo {
    /// Returns the path to the contract file if provided.
    pub fn path(&self) -> Option<PathBuf> {
        match self {
            Self::Path(path) => Some(path.clone()),
            Self::ContractInfo(info) => info.path.as_ref().map(PathBuf::from),
        }
    }

    /// Returns the contract name if provided.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Path(_) => None,
            Self::ContractInfo(info) => Some(&info.name),
        }
    }
}

impl FromStr for PathOrContractInfo {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self> {
        if let Ok(contract) = CompilerContractInfo::from_str(s) {
            return Ok(Self::ContractInfo(contract));
        }
        let path = PathBuf::from(s);
        if path.extension().is_some_and(|ext| ext == "sol" || ext == "vy") {
            return Ok(Self::Path(path));
        }
        Err(eyre::eyre!("Invalid contract identifier, file is not *.sol or *.vy: {}", s))
    }
}

impl std::fmt::Debug for PathOrContractInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(path) => write!(f, "Path({})", path.display()),
            Self::ContractInfo(info) => {
                write!(f, "ContractInfo({info})")
            }
        }
    }
}

/// Describes the Solidity library dependencies in creation or runtime bytecode, if any.
///
/// Includes cached artifacts and permits internal libraries and dynamic test linking helpers.
fn linked_libraries_error<C: Compiler<CompilerContract = Contract>>(
    output: &ProjectCompileOutput<C>,
    root: &Path,
) -> Option<String> {
    let violations = output
        .artifact_ids()
        .flat_map(|(id, artifact)| {
            let contract = id.with_stripped_file_prefixes(root).identifier();
            artifact.all_link_references().into_iter().flat_map(move |(file, libs)| {
                let contract = contract.clone();
                libs.into_keys().map(move |lib| format!("{contract} links {file}:{lib}"))
            })
        })
        .collect::<BTreeSet<_>>();
    (!violations.is_empty()).then(|| {
        format!(
            "Linked libraries are not allowed (`allow_linked_libraries = false`):\n  {}\n\
             help: make the library functions `internal` to inline them, or set \
             `allow_linked_libraries = true`",
            violations.into_iter().collect::<Vec<_>>().join("\n  ")
        )
    })
}

/// Appends a linked-library policy error to solc or multi-compiler diagnostics.
fn push_linked_libraries_error<C: Compiler<CompilerContract = Contract>>(
    output: &mut ProjectCompileOutput<C>,
    message: String,
) {
    let error = SolcError {
        source_location: None,
        secondary_source_locations: Vec::new(),
        r#type: "LinkedLibraryError".to_string(),
        component: "foundry".to_string(),
        severity: Severity::Error,
        error_code: None,
        message,
        formatted_message: None,
    };
    // CompilationError has no constructor; downcast to the concrete diagnostics instead.
    let errors = &mut output.output_mut().errors as &mut dyn Any;
    if let Some(errors) = errors.downcast_mut::<Vec<SolcError>>() {
        errors.push(error);
    } else if let Some(errors) = errors.downcast_mut::<Vec<MultiCompilerError>>() {
        errors.push(MultiCompilerError::Solc(error));
    }
}

/// Recognizes only solc code-generation failures that DTL's source rewrites can introduce.
/// Unrelated errors (including other languages) must not trigger a retry.
fn dynamic_test_linking_failure<E: CompilationError>(errors: &[E]) -> Option<&'static str> {
    let mut reason = None;
    for error in errors.iter().filter(|error| error.is_error()) {
        // CompilationError has no structured message/type accessors. Inspect the concrete solc
        // diagnostic rather than its Display output, which can include source snippets.
        let error = error as &dyn Any;
        let error = if let Some(error) = error.downcast_ref::<SolcError>() {
            error
        } else if let Some(MultiCompilerError::Solc(error)) =
            error.downcast_ref::<MultiCompilerError>()
        {
            error
        } else {
            return None;
        };
        reason = Some(match error.r#type.as_str() {
            "YulException" if error.message.contains("too deep in the stack") => "stack too deep",
            "CompilerError" if error.message.contains("Stack too deep") => "stack too deep",
            "InternalCompilerError"
                if error.message.contains("Tag too large for reserved space") =>
            {
                "Tag too large for reserved space"
            }
            _ => return None,
        });
    }
    reason
}

/// Finds root aliases that would resolve an import currently restricted to dependency contexts.
fn root_remapping_candidates<L: Clone>(
    paths: &ProjectPathsConfig<L>,
    import: &Path,
    file: &Path,
) -> BTreeMap<String, BTreeSet<String>> {
    let file = paths.root.join(file);
    let mut candidates = BTreeMap::<_, BTreeSet<_>>::new();
    let Some(cwd) = file.parent() else { return candidates };
    if !file.starts_with(&paths.root)
        || paths.libraries.iter().any(|lib| file.starts_with(paths.root.join(lib)))
        // An applicable root or CLI mapping remains authoritative, even if its target is missing.
        || paths.resolve_library_import(cwd, import).is_some()
    {
        return candidates;
    }

    let mut candidate_paths = paths.clone();
    candidate_paths.libraries.clear();
    for remapping in &paths.remappings {
        if let Some(context) = &remapping.context
            && paths
                .libraries
                .iter()
                .any(|lib| paths.root.join(context).starts_with(paths.root.join(lib)))
        {
            let mut global = remapping.clone();
            global.context = None;
            candidate_paths.remappings = vec![global.clone()];
            if candidate_paths
                .resolve_library_import(cwd, import)
                .is_some_and(|target| target.is_file())
            {
                candidates.entry(global.name.clone()).or_default().insert(
                    RelativeRemapping::new(global, &paths.root).to_relative_remapping().to_string(),
                );
            }
        }
    }
    candidates
}

/// Adds actionable alias migration help to existing import errors, including JSON diagnostics.
fn add_root_remapping_diagnostics<C: Compiler<CompilerContract = Contract>>(
    output: &mut ProjectCompileOutput<C>,
    paths: &ProjectPathsConfig<C::Language>,
) {
    if !output.output().errors.iter().any(|error| error.error_code() == Some(6275)) {
        return;
    }

    let mut suggestions = BTreeMap::<_, BTreeMap<_, BTreeSet<_>>>::new();
    for (import, file) in output.graph().unresolved_imports() {
        for (alias, mappings) in root_remapping_candidates(paths, import, file) {
            suggestions
                .entry(paths.root.join(file))
                .or_default()
                .entry(alias)
                .or_default()
                .extend(mappings);
        }
    }

    for error in &mut output.output_mut().errors {
        // CompilationError has no mutable diagnostic accessors; reuse concrete solc errors.
        let error = error as &mut dyn Any;
        let error = if error.is::<SolcError>() {
            error.downcast_mut::<SolcError>().unwrap()
        } else if let Some(MultiCompilerError::Solc(error)) =
            error.downcast_mut::<MultiCompilerError>()
        {
            error
        } else {
            continue;
        };
        if error.error_code == Some(6275)
            && let Some(location) = &error.source_location
            && let Some(aliases) = suggestions.remove(&paths.root.join(&location.file))
        {
            for (alias, mappings) in aliases {
                let action = if mappings.len() == 1 {
                    "add a remapping to foundry.toml or remappings.txt"
                } else {
                    "choose a root remapping in foundry.toml or remappings.txt"
                };
                let help = format!(
                    "help: this file imports the alias {alias:?}, which is scoped to a dependency. \
                     To use it from the root project, {action}:\n      {}",
                    mappings.into_iter().collect::<Vec<_>>().join("\n      ")
                );
                error.secondary_source_locations.push(SecondarySourceLocation {
                    file: None,
                    start: None,
                    end: None,
                    message: Some(help.clone()),
                });
                // Keep the original message intact so text rendering does not print help twice.
                let message = error.formatted_message.as_mut().unwrap_or(&mut error.message);
                message.truncate(message.trim_end().len());
                message.push_str("\n\n");
                message.push_str(&help);
                message.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_block_explorers::contract::SourceCodeMetadata;
    use foundry_compilers::multi::MultiCompilerLanguage;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[cfg(windows)]
    use std::process::Command;

    #[test]
    fn parse_contract_identifiers() {
        let t = ["src/Counter.sol", "src/Counter.sol:Counter", "Counter"];

        let i1 = PathOrContractInfo::from_str(t[0]).unwrap();
        assert_eq!(i1, PathOrContractInfo::Path(PathBuf::from(t[0])));

        let i2 = PathOrContractInfo::from_str(t[1]).unwrap();
        assert_eq!(
            i2,
            PathOrContractInfo::ContractInfo(CompilerContractInfo {
                path: Some("src/Counter.sol".to_string()),
                name: "Counter".to_string()
            })
        );

        let i3 = PathOrContractInfo::from_str(t[2]).unwrap();
        assert_eq!(
            i3,
            PathOrContractInfo::ContractInfo(CompilerContractInfo {
                path: None,
                name: "Counter".to_string()
            })
        );
    }

    #[test]
    fn size_report_uses_configured_limits() {
        let mut contracts = BTreeMap::new();
        contracts.insert(
            "LargeContract".to_string(),
            ContractInfo { runtime_size: 30_000, init_size: 60_000, is_dev_contract: false },
        );

        let default_report =
            SizeReport { contracts: contracts.clone(), limits: ContractSizeLimits::default() };
        assert!(default_report.exceeds_runtime_size_limit());
        assert!(default_report.exceeds_initcode_size_limit());

        let custom_report =
            SizeReport { contracts, limits: ContractSizeLimits::new(131_072, 262_144) };
        assert!(!custom_report.exceeds_runtime_size_limit());
        assert!(!custom_report.exceeds_initcode_size_limit());
        let output: serde_json::Value =
            serde_json::from_str(&custom_report.format_json_output()).unwrap();
        assert_eq!(
            output,
            serde_json::json!({
                "LargeContract": {
                    "runtime_size": 30000,
                    "init_size": 60000,
                    "runtime_margin": 101072,
                    "init_margin": 202144,
                }
            })
        );
    }

    #[test]
    fn contract_size_limits_derive_initcode_limit_from_runtime_limit() {
        assert_eq!(
            ContractSizeLimits::with_runtime_limit(50_000),
            ContractSizeLimits::new(50_000, 100_000)
        );
    }

    #[test]
    fn contract_size_limits_follow_evm_spec() {
        assert_eq!(ContractSizeLimits::for_spec_id(SpecId::OSAKA), ContractSizeLimits::default());
        assert_eq!(
            ContractSizeLimits::for_spec_id(SpecId::AMSTERDAM),
            ContractSizeLimits::new(65_536, 131_072)
        );
    }

    #[test]
    fn sanitized_remapping_paths_stay_inside_the_root() {
        let root = Path::new("/tmp/sources");
        for path in ["../../../etc", "a/../../../etc", "/etc", "./a/../b"] {
            let joined = root.join(sanitize_relative_path(Path::new(path)));
            assert!(
                joined.starts_with(root) && !joined.components().any(|c| c == Component::ParentDir),
                "{path} escaped the root: {}",
                joined.display()
            );
        }

        for (path, expected) in [("src/../lib/", "lib"), ("./a/../b", "b"), ("a/../../lib", "lib")]
        {
            assert_eq!(sanitize_relative_path(Path::new(path)), Path::new(expected));
        }

        // Ordinary relative paths are left alone.
        assert_eq!(
            sanitize_relative_path(Path::new("lib/openzeppelin")),
            Path::new("lib/openzeppelin")
        );
    }

    #[test]
    fn safe_contract_name() {
        for name in ["Example", "Contract_1", "Ünïcode"] {
            assert!(ensure_safe_contract_name(name).is_ok(), "{name:?}");
        }
        for name in ["", ".", "..", ".git", "a/b", "/abs"] {
            assert!(ensure_safe_contract_name(name).is_err(), "{name:?}");
        }

        #[cfg(target_os = "macos")]
        assert!(ensure_safe_contract_name("\u{200c}.git").is_err());
    }

    #[test]
    fn hfs_ignored_prefixes_before_dot_are_detected() {
        for codepoint in [
            0x200c, 0x200d, 0x200e, 0x200f, 0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x206a, 0x206b,
            0x206c, 0x206d, 0x206e, 0x206f, 0xfeff,
        ] {
            let prefix = char::from_u32(codepoint).unwrap();
            assert!(has_hfs_ignored_prefix_before_dot(&format!("{prefix}.git")));
        }
        assert!(has_hfs_ignored_prefix_before_dot("\u{200c}\u{202e}.git"));
        assert!(!has_hfs_ignored_prefix_before_dot("Ünïcode"));
        assert!(!has_hfs_ignored_prefix_before_dot("Example"));
    }

    #[test]
    fn safe_windows_path_component() {
        for component in ["Example", "My Contract.sol", "Ünïcode"] {
            assert!(is_safe_windows_path_component(component), "{component:?}");
        }
        for component in [
            "name.",
            "name ",
            "file:stream",
            "NUL.sol",
            "NUL .sol",
            "CON .txt",
            "COM1 .sol",
            "LPT1 .sol",
            "COM1",
            "LPT³.txt",
            "git~1",
            "GIT~1",
        ] {
            assert!(!is_safe_windows_path_component(component), "{component:?}");
        }
    }

    fn explorer_metadata(contract_name: &str, sources: &[(&str, &str)]) -> Metadata {
        Metadata {
            source_code: SourceCodeMetadata::Sources(
                sources.iter().map(|(path, content)| ((*path).into(), (*content).into())).collect(),
            ),
            abi: "[]".into(),
            contract_name: contract_name.into(),
            compiler_version: "invalid".into(),
            optimization_used: 0,
            runs: 0,
            constructor_arguments: Default::default(),
            evm_version: "Default".into(),
            library: String::new(),
            license_type: String::new(),
            proxy: 0,
            implementation: None,
            swarm_source: String::new(),
        }
    }

    #[test]
    fn etherscan_sources_validate_entire_reply_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("sources");
        let valid = explorer_metadata("Valid", &[("Contract.sol", "valid")]);
        for (invalid, expected) in [
            (
                explorer_metadata(".git", &[("config", "invalid")]),
                "invalid contract name from explorer: \".git\"".to_string(),
            ),
            (
                explorer_metadata("Invalid", &[("", "invalid")]),
                format!("invalid source path from explorer: {:?}", Path::new("Invalid").join("")),
            ),
        ] {
            let err = write_etherscan_sources(&[valid.clone(), invalid], &target).unwrap_err();
            assert_eq!(err.to_string(), expected);
            assert!(!target.exists());
        }
    }

    #[test]
    fn etherscan_sources_preserve_sanitized_paths() {
        let temp = tempfile::tempdir().unwrap();
        let metadata = [
            explorer_metadata(
                "Example",
                &[("/contracts/Example.sol", "source"), ("../../README", "readme")],
            ),
            explorer_metadata("Other", &[("Other.sol", "other source")]),
            explorer_metadata("Example", &[("Contract", "extensionless source")]),
            explorer_metadata("Example", &[("Contract.sol", "Solidity source")]),
        ];
        write_etherscan_sources(&metadata, temp.path()).unwrap();
        for (path, expected) in [
            ("Example/contracts/Example.sol", "source"),
            ("Example/README.sol", "readme"),
            ("Other/Other.sol", "other source"),
            ("Example/Contract", "extensionless source"),
            ("Example/Contract.sol", "Solidity source"),
        ] {
            assert_eq!(std::fs::read_to_string(temp.path().join(path)).unwrap(), expected);
        }

        let source = temp.path().join("Example/contracts/Example.sol");
        std::fs::write(&source, "stale source").unwrap();
        write_etherscan_sources(&metadata, temp.path()).unwrap();
        assert_eq!(std::fs::read_to_string(source).unwrap(), "source");
    }

    #[cfg(unix)]
    #[test]
    fn etherscan_sources_reject_symlinked_destinations() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("output");
        let git_dir = target.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        let git_config = git_dir.join("config");
        std::fs::write(&git_config, "original config").unwrap();
        symlink(&git_dir, target.join("Example")).unwrap();

        let metadata =
            explorer_metadata("Example", &[("config", "replacement"), ("config.sol", "companion")]);
        let valid = explorer_metadata("Safe", &[("Contract.sol", "valid")]);
        assert!(write_etherscan_sources(&[valid, metadata], &target).is_err());
        assert_eq!(std::fs::read_to_string(&git_config).unwrap(), "original config");
        assert!(!target.join("Safe/Contract.sol").exists());

        let target = temp.path().join("nested-output");
        let git_dir = target.join(".git");
        let contract_dir = target.join("Example");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::create_dir_all(&contract_dir).unwrap();
        let git_config = git_dir.join("config");
        std::fs::write(&git_config, "original nested config").unwrap();
        symlink(&git_dir, contract_dir.join(".git")).unwrap();

        let metadata = explorer_metadata(
            "Example",
            &[(".git/config", "replacement"), (".git/config.sol", "companion")],
        );
        assert!(write_etherscan_sources(&[metadata], &target).is_err());
        assert_eq!(std::fs::read_to_string(&git_config).unwrap(), "original nested config");

        let target = temp.path().join("file-output");
        let git_dir = target.join(".git");
        let contract_dir = target.join("Example");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::create_dir_all(&contract_dir).unwrap();
        let git_config = git_dir.join("config");
        std::fs::write(&git_config, "original file config").unwrap();
        symlink(&git_config, contract_dir.join("config")).unwrap();

        let metadata =
            explorer_metadata("Example", &[("config", "replacement"), ("config.sol", "companion")]);
        assert!(write_etherscan_sources(&[metadata], &target).is_err());
        assert_eq!(std::fs::read_to_string(&git_config).unwrap(), "original file config");

        // Extensionless sources are written to `.sol` unless a companion already uses that path.
        std::fs::remove_file(contract_dir.join("config")).unwrap();
        symlink(&git_config, contract_dir.join("config.sol")).unwrap();
        let metadata = explorer_metadata("Example", &[("config", "replacement")]);
        assert!(write_etherscan_sources(&[metadata], &target).is_err());
        assert_eq!(std::fs::read_to_string(&git_config).unwrap(), "original file config");
    }

    #[test]
    fn etherscan_project_rejects_invalid_metadata_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let external = temp.path().join("external");
        std::fs::create_dir(&external).unwrap();
        let external_source = external.join("Probe.sol");
        std::fs::write(&external_source, [0xff]).unwrap();
        let target = temp.path().join("sources");
        std::fs::create_dir(&target).unwrap();
        let contract_name = external.to_str().unwrap();
        let metadata = explorer_metadata(contract_name, &[("Example.sol", "replacement")]);
        let err = etherscan_project(&metadata, &target).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("invalid contract name from explorer: {contract_name:?}")
        );
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(std::fs::read(&external_source).unwrap(), [0xff]);

        let metadata = explorer_metadata("Example", &[("", "replacement")]);
        let err = etherscan_project(&metadata, &target).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("invalid source path from explorer: {:?}", Path::new("Example").join(""))
        );
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
    }

    #[test]
    fn root_remapping_candidates_respect_targets_choices_and_importers() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir_all(root.join("lib/a/src")).unwrap();
        fs::write(root.join("lib/a/src/Target.sol"), "contract Target {}\n").unwrap();
        let root = dunce::canonicalize(root).unwrap();
        let paths = ProjectPathsConfig::builder()
            .root(&root)
            .lib(root.join("lib"))
            .remapping(Remapping::from_str("lib/a/:@alias/=lib/a/src/").unwrap())
            .remapping(Remapping::from_str("lib/a/:File.sol=lib/a/src/Target.sol").unwrap())
            .build::<MultiCompilerLanguage>()
            .unwrap();
        let file = root.join("src/Root.sol");
        let import = Path::new("@alias/Target.sol");

        assert_eq!(
            root_remapping_candidates(&paths, Path::new("File.sol"), &file),
            BTreeMap::from([(
                "File.sol".to_string(),
                BTreeSet::from(["File.sol=lib/a/src/Target.sol".to_string()])
            )])
        );

        for (import, file) in [
            (Path::new("@alias/Missing.sol"), file.clone()),
            (import, root.join("lib/a/src/Target.sol")),
            (import, temp.path().join("Outside.sol")),
        ] {
            assert_eq!(root_remapping_candidates(&paths, import, &file), BTreeMap::new());
        }

        for explicit in ["@alias/=missing/", "src/:@alias/=missing/"] {
            let mut paths = paths.clone();
            paths.remappings.push(Remapping::from_str(explicit).unwrap());
            assert_eq!(root_remapping_candidates(&paths, import, &file), BTreeMap::new());
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires NTFS 8.3 name creation"]
    fn etherscan_sources_reject_hidden_short_name_aliases() {
        let temp = tempfile::tempdir().unwrap();
        // Force a different short-name suffix when 8.3 names are enabled.
        std::fs::create_dir(temp.path().join("git~1")).unwrap();
        let git_dir = temp.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        let git_config = git_dir.join("config");
        std::fs::write(&git_config, "original config").unwrap();
        let output = Command::new("cmd")
            .args(["/d", "/c", "for %I in (.git) do @echo %~snxI"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let short_path = String::from_utf8(output.stdout).unwrap();
        let name = Path::new(short_path.trim()).file_name().unwrap().to_str().unwrap();
        assert!(!name.starts_with('.'), "enable NTFS 8.3 name creation to run this regression");
        let metadata =
            explorer_metadata(name, &[("config", "replacement"), ("config.sol", "companion")]);
        let err = write_etherscan_sources(&[metadata], temp.path()).unwrap_err();
        assert_eq!(err.to_string(), "invalid contract name from explorer: \".git\"");
        assert_eq!(std::fs::read_to_string(&git_config).unwrap(), "original config");
    }
}
