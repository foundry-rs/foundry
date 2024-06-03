//! Support for compiling [foundry_compilers::Project]

use crate::{compact_to_contract, glob::GlobMatcher, term::SpinnerReporter, TestFunctionExt};
use comfy_table::{presets::ASCII_MARKDOWN, Attribute, Cell, CellAlignment, Color, Table};
use eyre::{Context, Result};
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::{
    artifacts::{BytecodeObject, ContractBytecodeSome, Libraries},
    compilers::{solc::SolcCompiler, Compiler},
    remappings::Remapping,
    report::{BasicStdoutReporter, NoReporter, Report},
    Artifact, ArtifactId, FileFilter, Project, ProjectBuilder, ProjectCompileOutput,
    ProjectPathsConfig, Solc, SolcConfig, SparseOutputFileFilter,
};
use foundry_linking::Linker;
use num_format::{Locale, ToFormattedString};
use rustc_hash::FxHashMap;
use std::{
    collections::{BTreeMap, HashMap},
    convert::Infallible,
    fmt::Display,
    io::IsTerminal,
    path::{Path, PathBuf},
    result,
    str::FromStr,
    time::Instant,
};

/// Builder type to configure how to compile a project.
///
/// This is merely a wrapper for [`Project::compile()`] which also prints to stdout depending on its
/// settings.
#[must_use = "ProjectCompiler does nothing unless you call a `compile*` method"]
pub struct ProjectCompiler<C: Compiler> {
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
    filter: Option<Box<dyn SparseOutputFileFilter<C::ParsedSource>>>,

    /// Extra files to include, that are not necessarily in the project's source dir.
    files: Vec<PathBuf>,
}

impl<C: Compiler> Default for ProjectCompiler<C> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Compiler> ProjectCompiler<C> {
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
    pub fn filter(mut self, filter: Box<dyn SparseOutputFileFilter<C::ParsedSource>>) -> Self {
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
    pub fn compile(
        mut self,
        project: &Project<C>,
    ) -> Result<ProjectCompileOutput<C::CompilationError>> {
        // TODO: Avoid process::exit
        if !project.paths.has_input_files() && self.files.is_empty() {
            println!("Nothing to compile");
            // nothing to do here
            std::process::exit(0);
        }

        // Taking is fine since we don't need these in `compile_with`.
        let filter = std::mem::take(&mut self.filter);
        let files = std::mem::take(&mut self.files);
        self.compile_with(|| {
            if !files.is_empty() {
                project.compile_files(files)
            } else if let Some(filter) = filter {
                project.compile_sparse(filter)
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
    /// ProjectCompiler::new().compile_with(|| Ok(prj.compile()?)).unwrap();
    /// ```
    #[instrument(target = "forge::compile", skip_all)]
    fn compile_with<F>(self, f: F) -> Result<ProjectCompileOutput<C::CompilationError>>
    where
        F: FnOnce() -> Result<ProjectCompileOutput<C::CompilationError>>,
    {
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

            let timer = Instant::now();
            let r = f();
            let elapsed = timer.elapsed();

            tracing::debug!("finished compiling in {:.3}s", elapsed.as_secs_f64());
            r
        })?;

        // need to drop the reporter here, so that the spinner terminates
        drop(reporter);

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
    fn handle_output(&self, output: &ProjectCompileOutput<C::CompilationError>) {
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

            let artifacts: BTreeMap<_, _> = output
                .artifact_ids()
                .filter(|(id, _)| {
                    // filter out forge-std specific contracts
                    !id.source.to_string_lossy().contains("/forge-std/src/")
                })
                .map(|(id, artifact)| (id.name, artifact))
                .collect();

            for (name, artifact) in artifacts {
                let size = deployed_contract_size(artifact).unwrap_or_default();

                let dev_functions =
                    artifact.abi.as_ref().map(|abi| abi.functions()).into_iter().flatten().filter(
                        |func| {
                            func.name.is_test() ||
                                func.name.eq("IS_TEST") ||
                                func.name.eq("IS_SCRIPT")
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

/// Contract source code and bytecode.
#[derive(Clone, Debug, Default)]
pub struct ContractSources {
    /// Map over artifacts' contract names -> vector of file IDs
    pub ids_by_name: HashMap<String, Vec<u32>>,
    /// Map over file_id -> source code
    pub sources_by_id: FxHashMap<u32, String>,
    /// Map over file_id -> contract name -> bytecode
    pub artifacts_by_id: FxHashMap<u32, HashMap<String, ContractBytecodeSome>>,
}

impl ContractSources {
    /// Collects the contract sources and artifacts from the project compile output.
    pub fn from_project_output(
        output: &ProjectCompileOutput,
        root: &Path,
        libraries: &Libraries,
    ) -> Result<ContractSources> {
        let linker = Linker::new(root, output.artifact_ids().collect());

        let mut sources = ContractSources::default();
        for (id, artifact) in output.artifact_ids() {
            if let Some(file_id) = artifact.id {
                let abs_path = root.join(&id.source);
                let source_code = std::fs::read_to_string(abs_path).wrap_err_with(|| {
                    format!("failed to read artifact source file for `{}`", id.identifier())
                })?;
                let linked = linker.link(&id, libraries)?;
                let contract = compact_to_contract(linked)?;
                sources.insert(&id, file_id, source_code, contract);
            } else {
                warn!(id = id.identifier(), "source not found");
            }
        }
        Ok(sources)
    }

    /// Inserts a contract into the sources.
    pub fn insert(
        &mut self,
        artifact_id: &ArtifactId,
        file_id: u32,
        source: String,
        bytecode: ContractBytecodeSome,
    ) {
        self.ids_by_name.entry(artifact_id.name.clone()).or_default().push(file_id);
        self.sources_by_id.insert(file_id, source);
        self.artifacts_by_id.entry(file_id).or_default().insert(artifact_id.name.clone(), bytecode);
    }

    /// Returns the source for a contract by file ID.
    pub fn get(&self, id: u32) -> Option<&String> {
        self.sources_by_id.get(&id)
    }

    /// Returns all sources for a contract by name.
    pub fn get_sources<'a>(
        &'a self,
        name: &'a str,
    ) -> Option<impl Iterator<Item = (u32, &'_ str, &'_ ContractBytecodeSome)>> {
        self.ids_by_name.get(name).map(|ids| {
            ids.iter().filter_map(|id| {
                Some((
                    *id,
                    self.sources_by_id.get(id)?.as_ref(),
                    self.artifacts_by_id.get(id)?.get(name)?,
                ))
            })
        })
    }

    /// Returns all (name, source, bytecode) sets.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str, &ContractBytecodeSome)> {
        self.artifacts_by_id
            .iter()
            .filter_map(|(id, artifacts)| {
                let source = self.sources_by_id.get(id)?;
                Some(
                    artifacts
                        .iter()
                        .map(move |(name, bytecode)| (name.as_ref(), source.as_ref(), bytecode)),
                )
            })
            .flatten()
    }
}

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
            Cell::new("Size (B)").add_attribute(Attribute::Bold).fg(Color::Blue),
            Cell::new("Margin (B)").add_attribute(Attribute::Bold).fg(Color::Blue),
        ]);

        // filters out non dev contracts (Test or Script)
        let contracts = self.contracts.iter().filter(|(_, c)| !c.is_dev_contract && c.size > 0);
        for (name, contract) in contracts {
            let margin = CONTRACT_SIZE_LIMIT as isize - contract.size as isize;
            let color = match contract.size {
                0..=17999 => Color::Reset,
                18000..=CONTRACT_SIZE_LIMIT => Color::Yellow,
                _ => Color::Red,
            };

            let locale = &Locale::en;
            table.add_row([
                Cell::new(name).fg(color),
                Cell::new(contract.size.to_formatted_string(locale))
                    .set_alignment(CellAlignment::Right)
                    .fg(color),
                Cell::new(margin.to_formatted_string(locale))
                    .set_alignment(CellAlignment::Right)
                    .fg(color),
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
pub fn compile_target<C: Compiler>(
    target_path: &Path,
    project: &Project<C>,
    quiet: bool,
) -> Result<ProjectCompileOutput<C::CompilationError>> {
    ProjectCompiler::<C>::new().quiet(quiet).files([target_path.into()]).compile(project)
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
pub fn etherscan_project(
    metadata: &Metadata,
    target_path: impl AsRef<Path>,
) -> Result<Project<SolcCompiler>> {
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
    let solc = Solc::find_or_install(&v)?;

    let compiler = SolcCompiler::Specific(solc);

    Ok(ProjectBuilder::<SolcCompiler>::default()
        .settings(SolcConfig::builder().settings(settings).build().settings)
        .paths(paths)
        .ephemeral()
        .no_artifacts()
        .build(compiler)?)
}

/// Bundles multiple `SkipBuildFilter` into a single `FileFilter`
#[derive(Clone, Debug)]
pub struct SkipBuildFilters {
    /// All provided filters.
    pub matchers: Vec<GlobMatcher>,
    /// Root of the project.
    pub project_root: PathBuf,
}

impl FileFilter for SkipBuildFilters {
    /// Only returns a match if _no_  exclusion filter matches
    fn is_match(&self, file: &Path) -> bool {
        self.matchers.iter().all(|matcher| {
            if !is_match_exclude(matcher, file) {
                false
            } else {
                file.strip_prefix(&self.project_root)
                    .map_or(true, |stripped| is_match_exclude(matcher, stripped))
            }
        })
    }
}

impl FileFilter for &SkipBuildFilters {
    fn is_match(&self, file: &Path) -> bool {
        (*self).is_match(file)
    }
}

impl SkipBuildFilters {
    /// Creates a new `SkipBuildFilters` from multiple `SkipBuildFilter`.
    pub fn new(
        filters: impl IntoIterator<Item = SkipBuildFilter>,
        project_root: PathBuf,
    ) -> Result<Self> {
        let matchers = filters.into_iter().map(|m| m.compile()).collect::<Result<_>>();
        matchers.map(|filters| Self { matchers: filters, project_root })
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
    fn new(s: &str) -> Self {
        match s {
            "test" | "tests" => SkipBuildFilter::Tests,
            "script" | "scripts" => SkipBuildFilter::Scripts,
            s => SkipBuildFilter::Custom(s.to_string()),
        }
    }

    /// Returns the pattern to match against a file
    fn file_pattern(&self) -> &str {
        match self {
            SkipBuildFilter::Tests => ".t.sol",
            SkipBuildFilter::Scripts => ".s.sol",
            SkipBuildFilter::Custom(s) => s.as_str(),
        }
    }

    fn compile(&self) -> Result<GlobMatcher> {
        self.file_pattern().parse().map_err(Into::into)
    }
}

impl FromStr for SkipBuildFilter {
    type Err = Infallible;

    fn from_str(s: &str) -> result::Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

/// Matches file only if the filter does not apply.
///
/// This returns the inverse of `file.name.contains(pattern) || matcher.is_match(file)`.
fn is_match_exclude(matcher: &GlobMatcher, path: &Path) -> bool {
    fn is_match(matcher: &GlobMatcher, path: &Path) -> Option<bool> {
        let file_name = path.file_name()?.to_str()?;
        Some(file_name.contains(matcher.as_str()) || matcher.is_match(path))
    }

    !is_match(matcher, path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_filter() {
        let tests = SkipBuildFilter::Tests.compile().unwrap();
        let scripts = SkipBuildFilter::Scripts.compile().unwrap();
        let custom = |s: &str| SkipBuildFilter::Custom(s.to_string()).compile().unwrap();

        let file = Path::new("A.t.sol");
        assert!(!is_match_exclude(&tests, file));
        assert!(is_match_exclude(&scripts, file));
        assert!(!is_match_exclude(&custom("A.t"), file));

        let file = Path::new("A.s.sol");
        assert!(is_match_exclude(&tests, file));
        assert!(!is_match_exclude(&scripts, file));
        assert!(!is_match_exclude(&custom("A.s"), file));

        let file = Path::new("/home/test/Foo.sol");
        assert!(!is_match_exclude(&custom("*/test/**"), file));

        let file = Path::new("/home/script/Contract.sol");
        assert!(!is_match_exclude(&custom("*/script/**"), file));
    }
}
