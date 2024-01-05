//! Support for compiling [foundry_compilers::Project]

use crate::{compact_to_contract, glob::GlobMatcher, term::SpinnerReporter, TestFunctionExt};
use comfy_table::{presets::ASCII_MARKDOWN, Attribute, Cell, Color, Table};
use eyre::Result;
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::{
    artifacts::{BytecodeObject, ContractBytecodeSome},
    remappings::Remapping,
    report::{BasicStdoutReporter, NoReporter, Report},
    Artifact, ArtifactId, FileFilter, Graph, Project, ProjectCompileOutput, ProjectPathsConfig,
    Solc, SolcConfig,
};
use std::{
    collections::{BTreeMap, HashMap},
    convert::Infallible,
    fmt::Display,
    io::IsTerminal,
    path::{Path, PathBuf},
    result,
    str::FromStr,
};

/// Builder type to configure how to compile a project.
///
/// This is merely a wrapper for [`Project::compile()`] which also prints to stdout depending on its
/// settings.
#[must_use = "this builder does nothing unless you call a `compile*` method"]
pub struct ProjectCompiler {
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

    /// Files to exclude.
    filter: Option<Box<dyn FileFilter>>,

    /// Extra files to include, that are not necessarily in the project's source dir.
    files: Vec<PathBuf>,
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
            verify: None,
            print_names: None,
            print_sizes: None,
            quiet: Some(crate::shell::verbosity().is_silent()),
            bail: None,
            filter: None,
            files: Vec::new(),
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

    /// Do not print anything at all if true. Overrides other `print` options.
    #[inline]
    pub fn quiet_if(mut self, maybe: bool) -> Self {
        if maybe {
            self.quiet = Some(true);
        }
        self
    }

    /// Sets whether to bail on compiler errors.
    #[inline]
    pub fn bail(mut self, yes: bool) -> Self {
        self.bail = Some(yes);
        self
    }

    /// Sets the filter to use.
    #[inline]
    pub fn filter(mut self, filter: Box<dyn FileFilter>) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Sets extra files to include, that are not necessarily in the project's source dir.
    #[inline]
    pub fn files(mut self, files: impl IntoIterator<Item = PathBuf>) -> Self {
        self.files.extend(files);
        self
    }

    /// Compiles the project.
    pub fn compile(mut self, project: &Project) -> Result<ProjectCompileOutput> {
        // Taking is fine since we don't need these in `compile_with`.
        let filter = std::mem::take(&mut self.filter);
        let files = std::mem::take(&mut self.files);
        self.compile_with(project, || {
            if !files.is_empty() {
                project.compile_files(files)
            } else if let Some(filter) = filter {
                project.compile_sparse(Box::new(move |file: &_| filter.is_match(file)))
            } else {
                project.compile()
            }
            .map_err(Into::into)
        })
    }

    /// Compiles the project with the given closure
    ///
    /// # Example
    ///
    /// ```ignore
    /// use foundry_common::compile::ProjectCompiler;
    /// let config = foundry_config::Config::load();
    /// let prj = config.project().unwrap();
    /// ProjectCompiler::new().compile_with(&prj, || Ok(prj.compile()?)).unwrap();
    /// ```
    #[instrument(target = "forge::compile", skip_all)]
    fn compile_with<F>(self, project: &Project, f: F) -> Result<ProjectCompileOutput>
    where
        F: FnOnce() -> Result<ProjectCompileOutput>,
    {
        // TODO: Avoid process::exit
        if !project.paths.has_input_files() {
            println!("Nothing to compile");
            // nothing to do here
            std::process::exit(0);
        }

        let quiet = self.quiet.unwrap_or(false);
        let bail = self.bail.unwrap_or(true);
        #[allow(clippy::collapsible_else_if)]
        let reporter = if quiet {
            Report::new(NoReporter::default())
        } else {
            if std::io::stdout().is_terminal() {
                Report::new(SpinnerReporter::spawn())
            } else {
                Report::new(BasicStdoutReporter::default())
            }
        };

        let output = foundry_compilers::report::with_scoped(&reporter, || {
            tracing::debug!("compiling project");

            let timer = std::time::Instant::now();
            let r = f();
            let elapsed = timer.elapsed();

            tracing::debug!("finished compiling in {:.3}s", elapsed.as_secs_f64());
            r
        })?;

        if bail && output.has_compiler_errors() {
            eyre::bail!("{output}")
        }

        if !quiet {
            if output.is_unchanged() {
                println!("No files changed, compilation skipped");
            } else {
                // print the compiler output / warnings
                println!("{output}");
            }

            self.handle_output(&output);
        }

        Ok(output)
    }

    /// If configured, this will print sizes or names
    fn handle_output(&self, output: &ProjectCompileOutput) {
        let print_names = self.print_names.unwrap_or(false);
        let print_sizes = self.print_sizes.unwrap_or(false);

        // print any sizes or names
        if print_names {
            let mut artifacts: BTreeMap<_, Vec<_>> = BTreeMap::new();
            for (name, (_, version)) in output.versioned_artifacts() {
                artifacts.entry(version).or_default().push(name);
            }
            for (version, names) in artifacts {
                println!(
                    "  compiler version: {}.{}.{}",
                    version.major, version.minor, version.patch
                );
                for name in names {
                    println!("    - {name}");
                }
            }
        }

        if print_sizes {
            // add extra newline if names were already printed
            if print_names {
                println!();
            }

            let mut size_report = SizeReport { contracts: BTreeMap::new() };
            let artifacts: BTreeMap<_, _> = output.artifacts().collect();
            for (name, artifact) in artifacts {
                let size = deployed_contract_size(artifact).unwrap_or_default();

                let dev_functions =
                    artifact.abi.as_ref().map(|abi| abi.functions()).into_iter().flatten().filter(
                        |&func| {
                            func.name.is_test() ||
                                func.name == "IS_TEST" ||
                                func.name == "IS_SCRIPT"
                        },
                    );

                let is_dev_contract = dev_functions.count() > 0;
                size_report.contracts.insert(name, ContractInfo { size, is_dev_contract });
            }

            println!("{size_report}");

            // TODO: avoid process::exit
            // exit with error if any contract exceeds the size limit, excluding test contracts.
            if size_report.exceeds_size_limit() {
                std::process::exit(1);
            }
        }
    }
}

/// Map over artifacts contract sources name -> file_id -> (source, contract)
#[derive(Clone, Debug, Default)]
pub struct ContractSources(pub HashMap<String, HashMap<u32, (String, ContractBytecodeSome)>>);

// https://eips.ethereum.org/EIPS/eip-170
const CONTRACT_SIZE_LIMIT: usize = 24576;

/// Contracts with info about their size
pub struct SizeReport {
    /// `contract name -> info`
    pub contracts: BTreeMap<String, ContractInfo>,
}

impl SizeReport {
    /// Returns the size of the largest contract, excluding test contracts.
    pub fn max_size(&self) -> usize {
        let mut max_size = 0;
        for contract in self.contracts.values() {
            if !contract.is_dev_contract && contract.size > max_size {
                max_size = contract.size;
            }
        }
        max_size
    }

    /// Returns true if any contract exceeds the size limit, excluding test contracts.
    pub fn exceeds_size_limit(&self) -> bool {
        self.max_size() > CONTRACT_SIZE_LIMIT
    }
}

impl Display for SizeReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let mut table = Table::new();
        table.load_preset(ASCII_MARKDOWN);
        table.set_header([
            Cell::new("Contract").add_attribute(Attribute::Bold).fg(Color::Blue),
            Cell::new("Size (kB)").add_attribute(Attribute::Bold).fg(Color::Blue),
            Cell::new("Margin (kB)").add_attribute(Attribute::Bold).fg(Color::Blue),
        ]);

        let contracts = self.contracts.iter().filter(|(_, c)| !c.is_dev_contract && c.size > 0);
        for (name, contract) in contracts {
            let margin = CONTRACT_SIZE_LIMIT as isize - contract.size as isize;
            let color = match contract.size {
                0..=17999 => Color::Reset,
                18000..=CONTRACT_SIZE_LIMIT => Color::Yellow,
                _ => Color::Red,
            };

            table.add_row([
                Cell::new(name).fg(color),
                Cell::new(contract.size as f64 / 1000.0).fg(color),
                Cell::new(margin as f64 / 1000.0).fg(color),
            ]);
        }

        writeln!(f, "{table}")?;
        Ok(())
    }
}

/// Returns the size of the deployed contract
pub fn deployed_contract_size<T: Artifact>(artifact: &T) -> Option<usize> {
    let bytecode = artifact.get_deployed_bytecode_object()?;
    let size = match bytecode.as_ref() {
        BytecodeObject::Bytecode(bytes) => bytes.len(),
        BytecodeObject::Unlinked(unlinked) => {
            // we don't need to account for placeholders here, because library placeholders take up
            // 40 characters: `__$<library hash>$__` which is the same as a 20byte address in hex.
            let mut size = unlinked.as_bytes().len();
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
    /// size of the contract in bytes
    pub size: usize,
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
pub fn compile_target_with_filter(
    target_path: &Path,
    project: &Project,
    quiet: bool,
    verify: bool,
    skip: Vec<SkipBuildFilter>,
) -> Result<ProjectCompileOutput> {
    let graph = Graph::resolve(&project.paths)?;

    // Checking if it's a standalone script, or part of a project.
    let mut compiler = ProjectCompiler::new().filter(Box::new(SkipBuildFilters(skip))).quiet(quiet);
    if !graph.files().contains_key(target_path) {
        if verify {
            eyre::bail!("You can only verify deployments from inside a project! Make sure it exists with `forge tree`.");
        }
        compiler = compiler.files([target_path.into()]);
    }
    compiler.compile(project)
}

/// Compiles an Etherscan source from metadata by creating a project.
/// Returns the artifact_id, the file_id, and the bytecode
pub async fn compile_from_source(
    metadata: &Metadata,
) -> Result<(ArtifactId, u32, ContractBytecodeSome)> {
    let root = tempfile::tempdir()?;
    let root_path = root.path();
    let project = etherscan_project(metadata, root_path)?;

    let project_output = project.compile()?;

    if project_output.has_compiler_errors() {
        eyre::bail!("{project_output}")
    }

    let (artifact_id, file_id, contract) = project_output
        .into_artifacts()
        .find(|(artifact_id, _)| artifact_id.name == metadata.contract_name)
        .map(|(aid, art)| {
            (aid, art.source_file().expect("no source file").id, art.into_contract_bytecode())
        })
        .ok_or_else(|| {
            eyre::eyre!(
                "Unable to find bytecode in compiled output for contract: {}",
                metadata.contract_name
            )
        })?;
    let bytecode = compact_to_contract(contract)?;

    root.close()?;

    Ok((artifact_id, file_id, bytecode))
}

/// Creates a [Project] from an Etherscan source.
pub fn etherscan_project(metadata: &Metadata, target_path: impl AsRef<Path>) -> Result<Project> {
    let target_path = dunce::canonicalize(target_path.as_ref())?;
    let sources_path = target_path.join(&metadata.contract_name);
    metadata.source_tree().write_to(&target_path)?;

    let mut settings = metadata.source_code.settings()?.unwrap_or_default();

    // make remappings absolute with our root
    for remapping in settings.remappings.iter_mut() {
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
    let v = format!("{}.{}.{}", v.major, v.minor, v.patch);
    let solc = Solc::find_or_install_svm_version(v)?;

    Ok(Project::builder()
        .solc_config(SolcConfig::builder().settings(settings).build())
        .no_auto_detect()
        .paths(paths)
        .solc(solc)
        .ephemeral()
        .no_artifacts()
        .build()?)
}

/// Bundles multiple `SkipBuildFilter` into a single `FileFilter`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipBuildFilters(pub Vec<SkipBuildFilter>);

impl FileFilter for SkipBuildFilters {
    /// Only returns a match if _no_  exclusion filter matches
    fn is_match(&self, file: &Path) -> bool {
        self.0.iter().all(|filter| filter.is_match(file))
    }
}

/// A filter that excludes matching contracts from the build
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipBuildFilter {
    /// Exclude all `.t.sol` contracts
    Tests,
    /// Exclude all `.s.sol` contracts
    Scripts,
    /// Exclude if the file matches
    Custom(String),
}

impl SkipBuildFilter {
    /// Returns the pattern to match against a file
    fn file_pattern(&self) -> &str {
        match self {
            SkipBuildFilter::Tests => ".t.sol",
            SkipBuildFilter::Scripts => ".s.sol",
            SkipBuildFilter::Custom(s) => s.as_str(),
        }
    }
}

impl<T: AsRef<str>> From<T> for SkipBuildFilter {
    fn from(s: T) -> Self {
        match s.as_ref() {
            "test" | "tests" => SkipBuildFilter::Tests,
            "script" | "scripts" => SkipBuildFilter::Scripts,
            s => SkipBuildFilter::Custom(s.to_string()),
        }
    }
}

impl FromStr for SkipBuildFilter {
    type Err = Infallible;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        Ok(s.into())
    }
}

impl FileFilter for SkipBuildFilter {
    /// Matches file only if the filter does not apply
    ///
    /// This is returns the inverse of `file.name.contains(pattern) || matcher.is_match(file)`
    fn is_match(&self, file: &Path) -> bool {
        fn exclude(file: &Path, pattern: &str) -> Option<bool> {
            let matcher: GlobMatcher = pattern.parse().unwrap();
            let file_name = file.file_name()?.to_str()?;
            Some(file_name.contains(pattern) || matcher.is_match(file.as_os_str().to_str()?))
        }

        !exclude(file, self.file_pattern()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_filter() {
        let file = Path::new("A.t.sol");
        assert!(!SkipBuildFilter::Tests.is_match(file));
        assert!(SkipBuildFilter::Scripts.is_match(file));
        assert!(!SkipBuildFilter::Custom("A.t".to_string()).is_match(file));

        let file = Path::new("A.s.sol");
        assert!(SkipBuildFilter::Tests.is_match(file));
        assert!(!SkipBuildFilter::Scripts.is_match(file));
        assert!(!SkipBuildFilter::Custom("A.s".to_string()).is_match(file));

        let file = Path::new("/home/test/Foo.sol");
        assert!(!SkipBuildFilter::Custom("*/test/**".to_string()).is_match(file));
        let file = Path::new("/home/script/Contract.sol");
        assert!(!SkipBuildFilter::Custom("*/script/**".to_string()).is_match(file));
    }
}
