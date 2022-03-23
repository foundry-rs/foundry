use crate::{opts::forge::ContractInfo, term};
use ethers::{
    abi::Abi,
    prelude::{
        artifacts::{CompactBytecode, CompactDeployedBytecode},
        report::NoReporter,
    },
    solc::cache::SolFilesCache,
};
use std::{collections::BTreeMap, path::PathBuf};

/// Common trait for all cli commands
pub trait Cmd: clap::Parser + Sized {
    type Output;
    fn run(self) -> eyre::Result<Self::Output>;
}

use ethers::solc::{artifacts::CompactContractBytecode, Artifact, Project, ProjectCompileOutput};

use foundry_utils::to_table;

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
pub fn compile(
    project: &Project,
    print_names: bool,
    print_sizes: bool,
) -> eyre::Result<ProjectCompileOutput> {
    if !project.paths.sources.exists() {
        eyre::bail!(
            r#"no contracts to compile, contracts folder "{}" does not exist.
Check the configured workspace settings:
{}
If you are in a subdirectory in a Git repository, try adding `--root .`"#,
            project.paths.sources.display(),
            project.paths
        );
    }

    let output = term::with_spinner_reporter(|| project.compile())?;

    if output.has_compiler_errors() {
        eyre::bail!(output.to_string())
    } else if output.is_unchanged() {
        println!("No files changed, compilation skipped");
    } else {
        // print the compiler output / warnings
        println!("{}", output);

        // print any sizes or names
        if print_names {
            let compiled_contracts = output.compiled_contracts_by_compiler_version();
            for (version, contracts) in compiled_contracts.into_iter() {
                println!(
                    "  compiler version: {}.{}.{}",
                    version.major, version.minor, version.patch
                );
                for (name, _) in contracts {
                    println!("    - {}", name);
                }
            }
        }
        if print_sizes {
            // add extra newline if names were already printed
            if print_names {
                println!();
            }
            let compiled_contracts = output.compiled_contracts_by_compiler_version();
            let mut sizes = BTreeMap::new();
            for (_, contracts) in compiled_contracts.into_iter() {
                for (name, contract) in contracts {
                    let size = contract
                        .get_bytecode_bytes()
                        .map(|bytes| bytes.0.len())
                        .unwrap_or_default();
                    sizes.insert(name, size);
                }
            }
            let json = serde_json::to_value(&sizes)?;
            println!("name             size (bytes)");
            println!("-----------------------------");
            println!("{}", to_table(json));
        }
    }

    Ok(output)
}

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
/// Doesn't print anything to stdout, thus is "suppressed".
pub fn suppress_compile(project: &Project) -> eyre::Result<ProjectCompileOutput> {
    if !project.paths.sources.exists() {
        eyre::bail!(
            r#"no contracts to compile, contracts folder "{}" does not exist.
Check the configured workspace settings:
{}
If you are in a subdirectory in a Git repository, try adding `--root .`"#,
            project.paths.sources.display(),
            project.paths
        );
    }

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
pub fn compile_files(project: &Project, files: Vec<PathBuf>) -> eyre::Result<ProjectCompileOutput> {
    let output = term::with_spinner_reporter(|| project.compile_files(files))?;

    if output.has_compiler_errors() {
        eyre::bail!(output.to_string())
    }
    println!("{}", output);
    Ok(output)
}

/// Given a project and its compiled artifacts, proceeds to return the ABI, Bytecode and
/// Runtime Bytecode of the given contract.
pub fn read_artifact(
    project: &Project,
    compiled: ProjectCompileOutput,
    contract: ContractInfo,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    Ok(match contract.path {
        Some(path) => get_artifact_from_path(project, path, contract.name)?,
        None => get_artifact_from_name(contract, compiled)?,
    })
}

/// Helper function for finding a contract by ContractName
// TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
// contract name?
fn get_artifact_from_name(
    contract: ContractInfo,
    compiled: ProjectCompileOutput,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    let mut has_found_contract = false;
    let mut contract_artifact = None;

    for (artifact_id, artifact) in compiled.into_artifacts() {
        if artifact_id.name == contract.name {
            if has_found_contract {
                eyre::bail!("contract with duplicate name. pass path")
            }
            has_found_contract = true;
            contract_artifact = Some(artifact);
        }
    }

    Ok(match contract_artifact {
        Some(artifact) => (
            artifact
                .abi
                .map(Into::into)
                .ok_or_else(|| eyre::Error::msg(format!("abi not found for {}", contract.name)))?,
            artifact.bytecode.ok_or_else(|| {
                eyre::Error::msg(format!("bytecode not found for {}", contract.name))
            })?,
            artifact.deployed_bytecode.ok_or_else(|| {
                eyre::Error::msg(format!("bytecode not found for {}", contract.name))
            })?,
        ),
        None => {
            eyre::bail!("could not find artifact")
        }
    })
}

/// Find using src/ContractSource.sol:ContractName
fn get_artifact_from_path(
    project: &Project,
    contract_path: String,
    contract_name: String,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    // Get sources from the requested location
    let abs_path = dunce::canonicalize(PathBuf::from(contract_path))?;

    let cache = SolFilesCache::read_joined(&project.paths)?;

    // Read the artifact from disk
    let artifact: CompactContractBytecode = cache.read_artifact(abs_path, &contract_name)?;

    Ok((
        artifact
            .abi
            .ok_or_else(|| eyre::Error::msg(format!("abi not found for {}", contract_name)))?,
        artifact
            .bytecode
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {}", contract_name)))?,
        artifact
            .deployed_bytecode
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {}", contract_name)))?,
    ))
}
