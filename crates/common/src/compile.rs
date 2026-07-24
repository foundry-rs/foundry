//! Support for compiling [foundry_compilers::Project]

use crate::{
    TestFunctionExt, preprocessor::DynamicTestLinkingPreprocessor, shell, term::SpinnerReporter,
};
use alloy_json_abi::JsonAbi;
use comfy_table::{Cell, Color, Table, modifiers::UTF8_ROUND_CORNERS, presets::ASCII_MARKDOWN};
use eyre::{OptionExt, Result};
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::{
    Artifact, Project, ProjectBuilder, ProjectCompileOutput, ProjectPathsConfig, SolcConfig,
    artifacts::{
        BytecodeObject, Contract, Source, output_selection::OutputSelection, remappings::Remapping,
    },
    compilers::{
        Compiler,
        solc::{Solc, SolcCompiler},
    },
    info::ContractInfo as CompilerContractInfo,
    multi::{MultiCompiler, MultiCompilerSettings},
    project::Preprocessor,
    report::{BasicStdoutReporter, NoReporter, Report},
    solc::SolcSettings,
};
use num_format::{Locale, ToFormattedString};
use solar::{
    ast::{Arena, ContractKind, ItemKind},
    interface::{Session, source_map::FileName},
    parse::Parser,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt::Display,
    io::IsTerminal,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Instant,
};

/// A Solar compiler instance, to grant syntactic and semantic analysis capabilities.
pub type Analysis = Arc<solar::sema::Compiler>;

const ABI_SOLC_ENV: &str = "FOUNDRY_ABI_SOLC";

fn replace_abi_compiler(project: &mut Project) {
    if std::env::var_os(ABI_SOLC_ENV).is_some() {
        return;
    }

    let Some(solar) = std::env::current_exe().ok().and_then(|exe| solar_path(&exe)) else {
        return;
    };

    let Some(SolcCompiler::Specific(solc)) = &mut project.compiler.solc else { return };
    solc.solc = solar;
}

fn solar_path(exe: &Path) -> Option<PathBuf> {
    let solar = exe.parent()?.join(format!("solar{}", std::env::consts::EXE_SUFFIX));
    solar.is_file().then_some(solar)
}

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

    /// Whether to bail on compiler errors.
    bail: Option<bool>,

    /// Whether to ignore the contract initcode size limit introduced by EIP-3860.
    ignore_eip_3860: bool,

    /// Contract size limits used when reporting compiled contract sizes.
    size_limits: ContractSizeLimits,

    /// Extra files to include, that are not necessarily in the project's source directory.
    files: Vec<PathBuf>,

    /// Whether to compile with dynamic linking tests and scripts.
    dynamic_test_linking: bool,
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
            bail: None,
            ignore_eip_3860: false,
            size_limits: ContractSizeLimits::default(),
            files: Vec::new(),
            dynamic_test_linking: false,
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

    /// Sets if tests should be dynamically linked.
    #[inline]
    pub const fn dynamic_test_linking(mut self, preprocess: bool) -> Self {
        self.dynamic_test_linking = preprocess;
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
        if !project.paths.has_input_files() && self.files.is_empty() {
            sh_println!("Nothing to compile")?;
            std::process::exit(0);
        }

        // Taking is fine since we don't need these in `compile_with`.
        let files = std::mem::take(&mut self.files);
        let preprocess = self.dynamic_test_linking;
        self.compile_with(|| {
            let sources = if files.is_empty() {
                project.paths.read_input_files()?
            } else {
                Source::read_all(files)?
            };

            let mut compiler =
                foundry_compilers::project::ProjectCompiler::with_sources(project, sources)?;
            if preprocess {
                compiler = compiler.with_preprocessor(DynamicTestLinkingPreprocessor);
            }
            compiler.compile().map_err(Into::into)
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

        let output = with_compilation_reporter(quiet, Some(self.project_root.clone()), || {
            tracing::debug!("compiling project");

            let timer = Instant::now();
            let r = f();
            let elapsed = timer.elapsed();

            tracing::debug!("finished compiling in {:.3}s", elapsed.as_secs_f64());
            r
        })?;

        if bail && output.has_compiler_errors() {
            eyre::bail!("{output}");
        }

        if !quiet {
            if !shell::is_json() {
                if output.is_unchanged() {
                    sh_println!("No files changed, compilation skipped")?;
                } else {
                    // print the compiler output / warnings
                    sh_println!("{output}")?;
                }
            }

            if !(shell::is_json() && output.has_compiler_errors()) {
                self.handle_output(&output)?;
            }
        }

        Ok(output)
    }

    /// If configured, this will print sizes or names
    fn handle_output<C: Compiler<CompilerContract = Contract>>(
        &self,
        output: &ProjectCompileOutput<C>,
    ) -> Result<()> {
        let print_names = self.print_names.unwrap_or(false);
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
                artifacts.entry(id.name.clone()).or_default().push((id.source.clone(), artifact));
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

                    let is_dev_contract = artifact
                        .abi
                        .as_ref()
                        .map(|abi| {
                            abi.functions().any(|f| {
                                f.test_function_kind().is_known()
                                    || matches!(f.name.as_str(), "IS_TEST" | "IS_SCRIPT")
                            })
                        })
                        .unwrap_or(false);

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

            sh_println!("{size_report}")?;

            let runtime_eip = if size_report.limits.runtime == CONTRACT_RUNTIME_SIZE_LIMIT {
                "EIP-170: "
            } else {
                ""
            };
            eyre::ensure!(
                !size_report.exceeds_runtime_size_limit(),
                "some contracts exceed the runtime size limit ({runtime_eip}{} bytes)",
                size_report.limits.runtime
            );
            // Check size limits only if not ignoring EIP-3860
            let initcode_eip = if size_report.limits.initcode == CONTRACT_INITCODE_SIZE_LIMIT {
                "EIP-3860: "
            } else {
                ""
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
const CONTRACT_RUNTIME_SIZE_LIMIT: usize = 24576;

// https://eips.ethereum.org/EIPS/eip-3860
const CONTRACT_INITCODE_SIZE_LIMIT: usize = 49152;

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
            table.load_preset(ASCII_MARKDOWN);
        } else {
            table.apply_modifier(UTF8_ROUND_CORNERS);
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
pub fn compile_abi_project(
    project: &mut Project,
    compiler: ProjectCompiler,
) -> Result<ProjectCompileOutput> {
    replace_abi_compiler(project);
    project.update_output_selection(|selection| {
        // Request ABI so compilers populate `contracts` without producing bytecode outputs.
        *selection = OutputSelection::common_output_selection(["abi".to_string()]);
    });
    compiler.compile(project)
}

/// Compiles the target contract requesting only ABI output and returns its ABI.
pub fn compile_target_abi(
    project: &mut Project<MultiCompiler>,
    target_path: &Path,
    target_name: &str,
) -> Result<JsonAbi> {
    let target_path = dunce::canonicalize(target_path)?;
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
    let target_path = dunce::canonicalize(target_path)?;
    let sources_path = target_path.join(&metadata.contract_name);
    metadata.source_tree().write_to(&target_path)?;

    let mut settings = metadata.settings()?;

    // make remappings absolute with our root
    for remapping in &mut settings.remappings {
        let new_path = sources_path.join(remapping.path.trim_start_matches('/'));
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
    #[expect(clippy::collapsible_else_if)]
    let reporter = if quiet || shell::is_json() {
        Report::new(NoReporter::default())
    } else {
        if std::io::stderr().is_terminal() {
            Report::new(SpinnerReporter::spawn(project_root))
        } else {
            Report::new(BasicStdoutReporter::default())
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn finds_solar_next_to_executable() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join(format!("forge{}", std::env::consts::EXE_SUFFIX));
        let solar = temp.path().join(format!("solar{}", std::env::consts::EXE_SUFFIX));

        assert!(solar_path(&exe).is_none());
        std::fs::File::create(&solar).unwrap();
        assert_eq!(solar_path(&exe), Some(solar));
    }
}
