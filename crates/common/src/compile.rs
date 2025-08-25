//! Support for compiling [foundry_compilers::Project]

use crate::{
    TestFunctionExt,
    preprocessor::DynamicTestLinkingPreprocessor,
    reports::{ReportKind, report_kind},
    shell,
    term::SpinnerReporter,
};
use comfy_table::{Cell, Color, Table, modifiers::UTF8_ROUND_CORNERS};
use eyre::Result;
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::{
    Artifact, Project, ProjectBuilder, ProjectCompileOutput, ProjectPathsConfig, SolcConfig,
    artifacts::{BytecodeObject, Contract, Source, remappings::Remapping},
    compilers::{
        Compiler,
        solc::{Solc, SolcCompiler},
    },
    info::ContractInfo as CompilerContractInfo,
    project::Preprocessor,
    report::{BasicStdoutReporter, NoReporter, Report},
    solc::SolcSettings,
};
use num_format::{Locale, ToFormattedString};
use std::{
    collections::BTreeMap,
    fmt::Display,
    io::IsTerminal,
    path::{Path, PathBuf},
    str::FromStr,
    time::Instant,
};

/// Builder type to configure how to compile a project.
///
/// This is merely a wrapper for [`Project::compile()`] which also prints to stdout depending on its
/// settings.
#[must_use = "ProjectCompiler does nothing unless you call a `compile*` method"]
pub struct ProjectCompiler {
    /// The root of the project.
    project_root: PathBuf,

    /// Whether we are going to verify the contracts after compilation.
    verify: Option<bool>,

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

    /// Extra files to include, that are not necessarily in the project's source dir.
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
            verify: None,
            print_names: None,
            print_sizes: None,
            quiet: Some(crate::shell::is_quiet()),
            bail: None,
            ignore_eip_3860: false,
            files: Vec::new(),
            dynamic_test_linking: false,
        }
    }

    /// Sets whether we are going to verify the contracts after compilation.
    #[inline]
    pub fn verify(mut self, yes: bool) -> Self {
        self.verify = Some(yes);
        self
    }

    /// Sets whether to print contract names.
    #[inline]
    pub fn print_names(mut self, yes: bool) -> Self {
        self.print_names = Some(yes);
        self
    }

    /// Sets whether to print contract sizes.
    #[inline]
    pub fn print_sizes(mut self, yes: bool) -> Self {
        self.print_sizes = Some(yes);
        self
    }

    /// Sets whether to print anything at all. Overrides other `print` options.
    #[inline]
    #[doc(alias = "silent")]
    pub fn quiet(mut self, yes: bool) -> Self {
        self.quiet = Some(yes);
        self
    }

    /// Sets whether to bail on compiler errors.
    #[inline]
    pub fn bail(mut self, yes: bool) -> Self {
        self.bail = Some(yes);
        self
    }

    /// Sets whether to ignore EIP-3860 initcode size limits.
    #[inline]
    pub fn ignore_eip_3860(mut self, yes: bool) -> Self {
        self.ignore_eip_3860 = yes;
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
    pub fn dynamic_test_linking(mut self, preprocess: bool) -> Self {
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
            let sources = if !files.is_empty() {
                Source::read_all(files)?
            } else {
                project.paths.read_input_files()?
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
    ///
    /// # Example
    ///
    /// ```ignore
    /// use foundry_common::compile::ProjectCompiler;
    /// let config = foundry_config::Config::load().unwrap();
    /// let prj = config.project().unwrap();
    /// ProjectCompiler::new().compile_with(|| Ok(prj.compile()?)).unwrap();
    /// ```
    fn compile_with<C: Compiler<CompilerContract = Contract>, F>(
        self,
        f: F,
    ) -> Result<ProjectCompileOutput<C>>
    where
        F: FnOnce() -> Result<ProjectCompileOutput<C>>,
    {
        let quiet = self.quiet.unwrap_or(false);
        let bail = self.bail.unwrap_or(true);

        let output = with_compilation_reporter(quiet, || {
            tracing::debug!("compiling project");

            let timer = Instant::now();
            let r = f();
            let elapsed = timer.elapsed();

            tracing::debug!("finished compiling in {:.3}s", elapsed.as_secs_f64());
            r
        })?;

        if bail && output.has_compiler_errors() {
            eyre::bail!("{output}")
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

            self.handle_output(&output)?;
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
                SizeReport { report_kind: report_kind(), contracts: BTreeMap::new() };

            let mut artifacts: BTreeMap<String, Vec<_>> = BTreeMap::new();
            for (id, artifact) in output.artifact_ids().filter(|(id, _)| {
                // filter out forge-std specific contracts
                !id.source.to_string_lossy().contains("/forge-std/src/")
            }) {
                artifacts.entry(id.name.clone()).or_default().push((id.source.clone(), artifact));
            }

            for (name, artifact_list) in artifacts {
                for (path, artifact) in &artifact_list {
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

                    let unique_name = if artifact_list.len() > 1 {
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

            eyre::ensure!(
                !size_report.exceeds_runtime_size_limit(),
                "some contracts exceed the runtime size limit \
                 (EIP-170: {CONTRACT_RUNTIME_SIZE_LIMIT} bytes)"
            );
            // Check size limits only if not ignoring EIP-3860
            eyre::ensure!(
                self.ignore_eip_3860 || !size_report.exceeds_initcode_size_limit(),
                "some contracts exceed the initcode size limit \
                 (EIP-3860: {CONTRACT_INITCODE_SIZE_LIMIT} bytes)"
            );
        }

        Ok(())
    }
}

// https://eips.ethereum.org/EIPS/eip-170
const CONTRACT_RUNTIME_SIZE_LIMIT: usize = 24576;

// https://eips.ethereum.org/EIPS/eip-3860
const CONTRACT_INITCODE_SIZE_LIMIT: usize = 49152;

/// Contracts with info about their size
pub struct SizeReport {
    /// What kind of report to generate.
    report_kind: ReportKind,
    /// `contract name -> info`
    pub contracts: BTreeMap<String, ContractInfo>,
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
        self.max_runtime_size() > CONTRACT_RUNTIME_SIZE_LIMIT
    }

    /// Returns true if any contract exceeds the initcode size limit, excluding dev contracts.
    pub fn exceeds_initcode_size_limit(&self) -> bool {
        self.max_init_size() > CONTRACT_INITCODE_SIZE_LIMIT
    }
}

impl Display for SizeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self.report_kind {
            ReportKind::Text => {
                writeln!(f, "\n{}", self.format_table_output())?;
            }
            ReportKind::JSON => {
                writeln!(f, "{}", self.format_json_output())?;
            }
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
                        "runtime_margin": CONTRACT_RUNTIME_SIZE_LIMIT as isize - contract.runtime_size as isize,
                        "init_margin": CONTRACT_INITCODE_SIZE_LIMIT as isize - contract.init_size as isize,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();

        serde_json::to_string(&contracts).unwrap()
    }

    fn format_table_output(&self) -> Table {
        let mut table = Table::new();
        table.apply_modifier(UTF8_ROUND_CORNERS);

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
        for (name, contract) in contracts {
            let runtime_margin =
                CONTRACT_RUNTIME_SIZE_LIMIT as isize - contract.runtime_size as isize;
            let init_margin = CONTRACT_INITCODE_SIZE_LIMIT as isize - contract.init_size as isize;

            let runtime_color = match contract.runtime_size {
                ..18_000 => Color::Reset,
                18_000..=CONTRACT_RUNTIME_SIZE_LIMIT => Color::Yellow,
                _ => Color::Red,
            };

            let init_color = match contract.init_size {
                ..36_000 => Color::Reset,
                36_000..=CONTRACT_INITCODE_SIZE_LIMIT => Color::Yellow,
                _ => Color::Red,
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
/// If `quiet` no solc related output will be emitted to stdout.
///
/// If `verify` and it's a standalone script, throw error. Only allowed for projects.
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

/// Creates a [Project] from an Etherscan source.
pub fn etherscan_project(
    metadata: &Metadata,
    target_path: impl AsRef<Path>,
) -> Result<Project<SolcCompiler>> {
    let target_path = dunce::canonicalize(target_path.as_ref())?;
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

    let v = metadata.compiler_version()?;
    let solc = Solc::find_or_install(&v)?;

    let compiler = SolcCompiler::Specific(solc);

    Ok(ProjectBuilder::<SolcCompiler>::default()
        .settings(SolcSettings {
            settings: SolcConfig::builder().settings(settings).build(),
            ..Default::default()
        })
        .paths(paths)
        .ephemeral()
        .no_artifacts()
        .build(compiler)?)
}

/// Configures the reporter and runs the given closure.
pub fn with_compilation_reporter<O>(quiet: bool, f: impl FnOnce() -> O) -> O {
    #[expect(clippy::collapsible_else_if)]
    let reporter = if quiet || shell::is_json() {
        Report::new(NoReporter::default())
    } else {
        if std::io::stdout().is_terminal() {
            Report::new(SpinnerReporter::spawn())
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
    /// Non-canoncalized path provided via CLI.
    Path(PathBuf),
    /// Contract info provided via CLI.
    ContractInfo(CompilerContractInfo),
}

impl PathOrContractInfo {
    /// Returns the path to the contract file if provided.
    pub fn path(&self) -> Option<PathBuf> {
        match self {
            Self::Path(path) => Some(path.to_path_buf()),
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
}
