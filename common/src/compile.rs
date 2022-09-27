//! Support for compiling [ethers::solc::Project]
use crate::{term, TestFunctionExt};
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, *};
use ethers_solc::{
    artifacts::{BytecodeObject, Contract, ContractBytecodeSome, Source, Sources},
    report::NoReporter,
    Artifact, ArtifactId, FileFilter, Graph, Project, ProjectCompileOutput, Solc,
};
use foundry_config::Config;
use semver::Version;
use std::{
    collections::BTreeMap,
    fmt::Display,
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

/// Helper type to configure how to compile a project
///
/// This is merely a wrapper for [Project::compile()] which also prints to stdout dependent on its
/// settings
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectCompiler {
    /// whether to also print the contract names
    print_names: bool,
    /// whether to also print the contract sizes
    print_sizes: bool,
}

impl ProjectCompiler {
    /// Create a new instance with the settings
    pub fn new(print_names: bool, print_sizes: bool) -> Self {
        Self { print_names, print_sizes }
    }

    /// Compiles the project with [`Project::compile()`]
    pub fn compile(self, project: &Project) -> eyre::Result<ProjectCompileOutput> {
        self.compile_with(project, |prj| Ok(prj.compile()?))
    }

    /// Compiles the project with [`Project::compile_parse()`] and the given filter.
    ///
    /// This will emit artifacts only for files that match the given filter.
    /// Files that do _not_ match the filter are given a pruned output selection and do not generate
    /// artifacts.
    pub fn compile_sparse<F: FileFilter + 'static>(
        self,
        project: &Project,
        filter: F,
    ) -> eyre::Result<ProjectCompileOutput> {
        self.compile_with(project, |prj| Ok(prj.compile_sparse(filter)?))
    }

    /// Compiles the project with the given closure
    ///
    /// # Example
    ///
    /// ```no_run
    /// use foundry_common::compile::ProjectCompiler;
    /// let config = foundry_config::Config::load();
    /// ProjectCompiler::default()
    ///     .compile_with(&config.project().unwrap(), |prj| Ok(prj.compile()?)).unwrap();
    /// ```
    #[tracing::instrument(target = "forge::compile", skip_all)]
    pub fn compile_with<F>(self, project: &Project, f: F) -> eyre::Result<ProjectCompileOutput>
    where
        F: FnOnce(&Project) -> eyre::Result<ProjectCompileOutput>,
    {
        if !project.paths.has_input_files() {
            println!("Nothing to compile");
            // nothing to do here
            std::process::exit(0);
        }

        let now = std::time::Instant::now();
        tracing::trace!("start compiling project");

        let output = term::with_spinner_reporter(|| f(project))?;

        let elapsed = now.elapsed();
        tracing::trace!(?elapsed, "finished compiling");

        if output.has_compiler_errors() {
            tracing::warn!("compiled with errors");
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("No files changed, compilation skipped");
            self.handle_output(&output);
        } else {
            // print the compiler output / warnings
            println!("{output}");

            self.handle_output(&output);
        }

        Ok(output)
    }

    /// If configured, this will print sizes or names
    fn handle_output(&self, output: &ProjectCompileOutput) {
        // print any sizes or names
        if self.print_names {
            let compiled_contracts = output.compiled_contracts_by_compiler_version();
            for (version, contracts) in compiled_contracts.into_iter() {
                println!(
                    "  compiler version: {}.{}.{}",
                    version.major, version.minor, version.patch
                );
                for (name, _) in contracts {
                    println!("    - {name}");
                }
            }
        }
        if self.print_sizes {
            // add extra newline if names were already printed
            if self.print_names {
                println!();
            }
            let compiled_contracts = output.compiled_contracts_by_compiler_version();
            let mut size_report = SizeReport { contracts: BTreeMap::new() };
            for (_, contracts) in compiled_contracts.into_iter() {
                for (name, contract) in contracts {
                    let size = deployed_contract_size(&contract).unwrap_or_default();

                    let dev_functions =
                        contract.abi.as_ref().unwrap().abi.functions().into_iter().filter(|func| {
                            func.name.is_test() ||
                                func.name.eq("IS_TEST") ||
                                func.name.eq("IS_SCRIPT")
                        });

                    let is_dev_contract = dev_functions.into_iter().count() > 0;
                    size_report.contracts.insert(name, ContractInfo { size, is_dev_contract });
                }
            }

            println!("{size_report}");

            // exit with error if any contract exceeds the size limit, excluding test contracts.
            if size_report.exceeds_size_limit() {
                std::process::exit(1);
            }
        }
    }
}

// https://eips.ethereum.org/EIPS/eip-170
const CONTRACT_SIZE_LIMIT: usize = 24576;

/// Contracts with info about their size
pub struct SizeReport {
    /// `<contract name>:info>`
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
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL).apply_modifier(UTF8_ROUND_CORNERS);
        table.set_header(vec![
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

            table.add_row(vec![
                Cell::new(name).fg(color),
                Cell::new(contract.size as f64 / 1000.0).fg(color),
                Cell::new(margin as f64 / 1000.0).fg(color),
            ]);
        }

        writeln!(f, "{}", table)?;
        Ok(())
    }
}

/// Returns the size of the deployed contract
pub fn deployed_contract_size(contract: &Contract) -> Option<usize> {
    let bytecode = contract.get_deployed_bytecode_object()?;
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
#[derive(Debug, Clone, Copy)]
pub struct ContractInfo {
    /// size of the contract in bytes
    pub size: usize,
    /// A development contract is either a Script or a Test contract.
    pub is_dev_contract: bool,
}

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
pub fn compile(
    project: &Project,
    print_names: bool,
    print_sizes: bool,
) -> eyre::Result<ProjectCompileOutput> {
    ProjectCompiler::new(print_names, print_sizes).compile(project)
}

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
/// Doesn't print anything to stdout, thus is "suppressed".
pub fn suppress_compile(project: &Project) -> eyre::Result<ProjectCompileOutput> {
    let output = ethers_solc::report::with_scoped(
        &ethers_solc::report::Report::new(NoReporter::default()),
        || project.compile(),
    )?;

    if output.has_compiler_errors() {
        eyre::bail!(output.to_string())
    }

    Ok(output)
}

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
/// Doesn't print anything to stdout, thus is "suppressed".
///
/// See [`Project::compile_sparse`]
pub fn suppress_compile_sparse<F: FileFilter + 'static>(
    project: &Project,
    filter: F,
) -> eyre::Result<ProjectCompileOutput> {
    let output = ethers_solc::report::with_scoped(
        &ethers_solc::report::Report::new(NoReporter::default()),
        || project.compile_sparse(filter),
    )?;

    if output.has_compiler_errors() {
        eyre::bail!(output.to_string())
    }

    Ok(output)
}

/// Compile a set of files not necessarily included in the `project`'s source dir
///
/// If `silent` no solc related output will be emitted to stdout
pub fn compile_files(
    project: &Project,
    files: Vec<PathBuf>,
    silent: bool,
) -> eyre::Result<ProjectCompileOutput> {
    let output = if silent {
        ethers_solc::report::with_scoped(
            &ethers_solc::report::Report::new(NoReporter::default()),
            || project.compile_files(files),
        )
    } else {
        term::with_spinner_reporter(|| project.compile_files(files))
    }?;

    if output.has_compiler_errors() {
        eyre::bail!(output.to_string())
    }
    if !silent {
        println!("{output}");
    }

    Ok(output)
}

/// Compiles target file path.
///
/// If `silent` no solc related output will be emitted to stdout.
///
/// If `verify` and it's a standalone script, throw error. Only allowed for projects.
///
/// **Note:** this expects the `target_path` to be absolute
pub fn compile_target(
    target_path: &Path,
    project: &Project,
    silent: bool,
    verify: bool,
) -> eyre::Result<ProjectCompileOutput> {
    let graph = Graph::resolve(&project.paths)?;

    // Checking if it's a standalone script, or part of a project.
    if graph.files().get(target_path).is_none() {
        if verify {
            eyre::bail!("You can only verify deployments from inside a project! Make sure it exists with `forge tree`.");
        }
        return compile_files(project, vec![target_path.to_path_buf()], silent)
    }

    if silent {
        suppress_compile(project)
    } else {
        compile(project, false, false)
    }
}

/// Compile from etherscan bytecode.
pub async fn compile_from_source(
    contract_name: String,
    source: String,
    // has the contract been optimized before submission to etherscan
    optimization: bool,
    runs: u32,
    version: String,
) -> eyre::Result<(ArtifactId, ContractBytecodeSome)> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, "{}", source.clone())?;

    let target_contract = dunce::canonicalize(file.path())?;
    let mut project = Config::default().ephemeral_no_artifacts_project()?;

    if optimization {
        project.solc_config.settings.optimizer.enable();
        project.solc_config.settings.optimizer.runs(runs as usize);
    } else {
        project.solc_config.settings.optimizer.disable();
    }

    project.solc = if let Some(solc) = Solc::find_svm_installed_version(&version)? {
        solc
    } else {
        let v: Version = version.trim_start_matches('v').parse()?;
        Solc::install(&Version::new(v.major, v.minor, v.patch)).await?
    };

    let mut sources = Sources::new();
    sources.insert(target_contract, Source { content: source });

    let project_output = project.compile_with_version(&project.solc, sources)?;

    if project_output.has_compiler_errors() {
        eyre::bail!(project_output.to_string())
    }

    let (artifact_id, bytecode) = project_output
        .into_contract_bytecodes()
        .filter_map(|(artifact_id, contract)| {
            if artifact_id.name != contract_name {
                None
            } else {
                Some((
                    artifact_id,
                    ContractBytecodeSome {
                        abi: contract.abi.unwrap(),
                        bytecode: contract.bytecode.unwrap().into(),
                        deployed_bytecode: contract.deployed_bytecode.unwrap().into(),
                    },
                ))
            }
        })
        .into_iter()
        .next()
        .expect("there should be a contract with bytecode");
    Ok((artifact_id, bytecode))
}
