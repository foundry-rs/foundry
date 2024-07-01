//! Support for compiling [foundry_compilers::Project]

use crate::{compact_to_contract, term::SpinnerReporter, TestFunctionExt};
use comfy_table::{presets::ASCII_MARKDOWN, Attribute, Cell, CellAlignment, Color, Table};
use eyre::{Context, Result};
use foundry_block_explorers::contract::Metadata;
use foundry_compilers::{
    artifacts::{remappings::Remapping, BytecodeObject, ContractBytecodeSome, Libraries, Source},
    compilers::{
        multi::MultiCompilerLanguage,
        solc::{Solc, SolcCompiler},
        Compiler,
    },
    report::{BasicStdoutReporter, NoReporter, Report},
    Artifact, Project, ProjectBuilder, ProjectCompileOutput, ProjectPathsConfig, SolcConfig,
};
use foundry_linking::Linker;
use num_format::{Locale, ToFormattedString};
use rustc_hash::FxHashMap;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    io::IsTerminal,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

/// Builder type to configure how to compile a project.
///
/// This is merely a wrapper for [`Project::compile()`] which also prints to stdout depending on its
/// settings.
#[must_use = "ProjectCompiler does nothing unless you call a `compile*` method"]
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

    /// Sets extra files to include, that are not necessarily in the project's source dir.
    #[inline]
    pub fn files(mut self, files: impl IntoIterator<Item = PathBuf>) -> Self {
        self.files.extend(files);
        self
    }

    /// Compiles the project.
    pub fn compile<C: Compiler>(mut self, project: &Project<C>) -> Result<ProjectCompileOutput<C>> {
        // TODO: Avoid process::exit
        if !project.paths.has_input_files() && self.files.is_empty() {
            println!("Nothing to compile");
            // nothing to do here
            std::process::exit(0);
        }

        // Taking is fine since we don't need these in `compile_with`.
        let files = std::mem::take(&mut self.files);
        self.compile_with(|| {
            let sources = if !files.is_empty() {
                Source::read_all(files)?
            } else {
                project.paths.read_input_files()?
            };

            foundry_compilers::project::ProjectCompiler::with_sources(project, sources)?
                .compile()
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
    fn compile_with<C: Compiler, F>(self, f: F) -> Result<ProjectCompileOutput<C>>
    where
        F: FnOnce() -> Result<ProjectCompileOutput<C>>,
    {
        let quiet = self.quiet.unwrap_or(false);
        let bail = self.bail.unwrap_or(true);

        let output = with_compilation_reporter(self.quiet.unwrap_or(false), || {
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
    fn handle_output<C: Compiler>(&self, output: &ProjectCompileOutput<C>) {
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

                let is_dev_contract = artifact
                    .abi
                    .as_ref()
                    .map(|abi| {
                        abi.functions().any(|f| {
                            f.test_function_kind().is_known() ||
                                matches!(f.name.as_str(), "IS_TEST" | "IS_SCRIPT")
                        })
                    })
                    .unwrap_or(false);
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

#[derive(Clone, Debug)]
pub struct SourceData {
    pub source: Arc<String>,
    pub language: MultiCompilerLanguage,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ArtifactData {
    pub bytecode: ContractBytecodeSome,
    pub build_id: String,
    pub file_id: u32,
}

/// Contract source code and bytecode data used for debugger.
#[derive(Clone, Debug, Default)]
pub struct ContractSources {
    /// Map over build_id -> file_id -> (source code, language)
    pub sources_by_id: HashMap<String, FxHashMap<u32, SourceData>>,
    /// Map over contract name -> Vec<(bytecode, build_id, file_id)>
    pub artifacts_by_name: HashMap<String, Vec<ArtifactData>>,
}

impl ContractSources {
    /// Collects the contract sources and artifacts from the project compile output.
    pub fn from_project_output(
        output: &ProjectCompileOutput,
        root: impl AsRef<Path>,
        libraries: Option<&Libraries>,
    ) -> Result<Self> {
        let mut sources = Self::default();
        sources.insert(output, root, libraries)?;
        Ok(sources)
    }

    pub fn insert<C: Compiler>(
        &mut self,
        output: &ProjectCompileOutput<C>,
        root: impl AsRef<Path>,
        libraries: Option<&Libraries>,
    ) -> Result<()>
    where
        C::Language: Into<MultiCompilerLanguage>,
    {
        let root = root.as_ref();
        let link_data = libraries.map(|libraries| {
            let linker = Linker::new(root, output.artifact_ids().collect());
            (linker, libraries)
        });

        for (id, artifact) in output.artifact_ids() {
            if let Some(file_id) = artifact.id {
                let artifact = if let Some((linker, libraries)) = link_data.as_ref() {
                    linker.link(&id, libraries)?.into_contract_bytecode()
                } else {
                    artifact.clone().into_contract_bytecode()
                };
                let bytecode = compact_to_contract(artifact.clone().into_contract_bytecode())?;

                self.artifacts_by_name.entry(id.name.clone()).or_default().push(ArtifactData {
                    bytecode,
                    build_id: id.build_id.clone(),
                    file_id,
                });
            } else {
                warn!(id = id.identifier(), "source not found");
            }
        }

        // Not all source files produce artifacts, so we are populating sources by using build
        // infos.
        let mut files: BTreeMap<PathBuf, Arc<String>> = BTreeMap::new();
        for (build_id, build) in output.builds() {
            for (source_id, path) in &build.source_id_to_path {
                let source_code = if let Some(source) = files.get(path) {
                    source.clone()
                } else {
                    let source = Source::read(path).wrap_err_with(|| {
                        format!("failed to read artifact source file for `{}`", path.display())
                    })?;
                    files.insert(path.clone(), source.content.clone());
                    source.content
                };

                self.sources_by_id.entry(build_id.clone()).or_default().insert(
                    *source_id,
                    SourceData {
                        source: source_code,
                        language: build.language.into(),
                        name: path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string(),
                    },
                );
            }
        }

        Ok(())
    }

    /// Returns all sources for a contract by name.
    pub fn get_sources(
        &self,
        name: &str,
    ) -> Option<impl Iterator<Item = (&ArtifactData, &SourceData)>> {
        self.artifacts_by_name.get(name).map(|artifacts| {
            artifacts.iter().filter_map(|artifact| {
                let source =
                    self.sources_by_id.get(artifact.build_id.as_str())?.get(&artifact.file_id)?;
                Some((artifact, source))
            })
        })
    }

    /// Returns all (name, bytecode, source) sets.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &ArtifactData, &SourceData)> {
        self.artifacts_by_name.iter().flat_map(|(name, artifacts)| {
            artifacts.iter().filter_map(|artifact| {
                let source =
                    self.sources_by_id.get(artifact.build_id.as_str())?.get(&artifact.file_id)?;
                Some((name.as_str(), artifact, source))
            })
        })
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
) -> Result<ProjectCompileOutput<C>> {
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

/// Configures the reporter and runs the given closure.
pub fn with_compilation_reporter<O>(quiet: bool, f: impl FnOnce() -> O) -> O {
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

    foundry_compilers::report::with_scoped(&reporter, f)
}
