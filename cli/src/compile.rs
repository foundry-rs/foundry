//! Support for compiling [ethers::solc::Project]

use crate::term;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, *};
use ethers::{
    prelude::Graph,
    solc::{report::NoReporter, Artifact, FileFilter, Project, ProjectCompileOutput},
};
use std::{
    collections::BTreeMap,
    fmt::Display,
    path::{Path, PathBuf},
};

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
pub fn compile(
    project: &Project,
    print_names: bool,
    print_sizes: bool,
) -> eyre::Result<ProjectCompileOutput> {
    ProjectCompiler::new(print_names, print_sizes).compile(project)
}

// https://eips.ethereum.org/EIPS/eip-170
const CONTRACT_SIZE_LIMIT: usize = 24576;

pub struct SizeReport {
    pub contracts: BTreeMap<String, ContractInfo>,
}

pub struct ContractInfo {
    pub size: usize,
    // A development contract is either a Script or a Test contract.
    pub is_dev_contract: bool,
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
    /// let config = foundry_config::Config::load();
    /// ProjectCompiler::default()
    ///     .compile_with(&config.project().unwrap(), |prj| Ok(prj.compile()?));
    /// ```
    pub fn compile_with<F>(self, project: &Project, f: F) -> eyre::Result<ProjectCompileOutput>
    where
        F: FnOnce(&Project) -> eyre::Result<ProjectCompileOutput>,
    {
        let ProjectCompiler { print_sizes, print_names } = self;

        if !project.paths.has_input_files() {
            println!("Nothing to compile");
            // nothing to do here
            std::process::exit(0);
        }

        let now = std::time::Instant::now();
        tracing::trace!(target : "forge::compile", "start compiling project");

        let output = term::with_spinner_reporter(|| f(project))?;

        let elapsed = now.elapsed();
        tracing::trace!(target : "forge::compile", "finished compiling after {:?}", elapsed);

        if output.has_compiler_errors() {
            tracing::warn!(target: "forge::compile", "compiled with errors");
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("No files changed, compilation skipped");
        } else {
            // print the compiler output / warnings
            println!("{output}");

            // print any sizes or names
            if print_names {
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
            if print_sizes {
                // add extra newline if names were already printed
                if print_names {
                    println!();
                }
                let compiled_contracts = output.compiled_contracts_by_compiler_version();
                let mut size_report = SizeReport { contracts: BTreeMap::new() };
                for (_, contracts) in compiled_contracts.into_iter() {
                    for (name, contract) in contracts {
                        let size = contract
                            .get_bytecode_bytes()
                            .map(|bytes| bytes.0.len())
                            .unwrap_or_default();

                        let dev_functions =
                            contract.abi.as_ref().unwrap().abi.functions().into_iter().filter(
                                |func| {
                                    func.name.starts_with("test") ||
                                        func.name.eq("IS_TEST") ||
                                        func.name.eq("IS_SCRIPT")
                                },
                            );

                        let is_dev_contract = dev_functions.into_iter().count() > 0;
                        size_report.contracts.insert(name, ContractInfo { size, is_dev_contract });
                    }
                }

                println!("{size_report}");

                // exit with error if any contract exceeds the size limit, excluding test contracts.
                let exit_status = if size_report.exceeds_size_limit() { 1 } else { 0 };
                std::process::exit(exit_status);
            }
        }

        Ok(output)
    }
}

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
/// Doesn't print anything to stdout, thus is "suppressed".
pub fn suppress_compile(project: &Project) -> eyre::Result<ProjectCompileOutput> {
    let output = ethers::solc::report::with_scoped(
        &ethers::solc::report::Report::new(NoReporter::default()),
        || project.compile(),
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
        ethers::solc::report::with_scoped(
            &ethers::solc::report::Report::new(NoReporter::default()),
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
