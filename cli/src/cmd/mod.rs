//! Subcommands for forge

pub mod build;
pub mod create;
pub mod remappings;
pub mod run;
pub mod snapshot;
pub mod test;
pub mod verify;

use crate::opts::forge::ContractInfo;
use ethers::{
    abi::Abi,
    prelude::Graph,
    solc::{
        artifacts::{Source, Sources},
        cache::SolFilesCache,
    },
};
use std::path::PathBuf;

/// Common trait for all cli commands
pub trait Cmd: clap::Parser + Sized {
    type Output;
    fn run(self) -> eyre::Result<Self::Output>;
}

use ethers::solc::{
    artifacts::BytecodeObject, MinimalCombinedArtifacts, Project, ProjectCompileOutput,
};

/// Compiles the provided [`Project`], throws if there's any compiler error and logs whether
/// compilation was successful or if there was a cache hit.
// TODO: Move this to ethers-solc.
pub fn compile(project: &Project) -> eyre::Result<ProjectCompileOutput<MinimalCombinedArtifacts>> {
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

    println!("compiling...");
    let output = project.compile()?;
    if output.has_compiler_errors() {
        eyre::bail!(output.to_string())
    } else if output.is_unchanged() {
        println!("no files changed, compilation skipped.");
    } else {
        println!("success.");
    }
    Ok(output)
}

/// Manually compile a project with added sources
pub fn manual_compile(
    project: &Project<MinimalCombinedArtifacts>,
    added_sources: Vec<PathBuf>,
) -> eyre::Result<ProjectCompileOutput<MinimalCombinedArtifacts>> {
    let mut sources = project.paths.read_input_files()?;
    sources.extend(Source::read_all_files(added_sources)?);
    println!("compiling...");
    if project.auto_detect {
        tracing::trace!("using solc auto detection to compile sources");
        let output = project.svm_compile(sources)?;
        if output.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(output.to_string())
        }
        return Ok(output)
    }

    let mut solc = project.solc.clone();
    if !project.allowed_lib_paths.is_empty() {
        solc = solc.arg("--allow-paths").arg(project.allowed_lib_paths.to_string());
    }

    let sources = Graph::resolve_sources(&project.paths, sources)?.into_sources();
    let output = project.compile_with_version(&solc, sources)?;
    if output.has_compiler_errors() {
        // return the diagnostics error back to the user.
        eyre::bail!(output.to_string())
    }
    Ok(output)
}

/// Given a project and its compiled artifacts, proceeds to return the ABI, Bytecode and
/// Runtime Bytecode of the given contract.
pub fn read_artifact(
    project: &Project,
    compiled: ProjectCompileOutput<MinimalCombinedArtifacts>,
    contract: ContractInfo,
) -> eyre::Result<(Abi, BytecodeObject, BytecodeObject)> {
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
    compiled: ProjectCompileOutput<MinimalCombinedArtifacts>,
) -> eyre::Result<(Abi, BytecodeObject, BytecodeObject)> {
    let mut has_found_contract = false;
    let mut contract_artifact = None;

    for (name, artifact) in compiled.into_artifacts() {
        // if the contract name
        let mut split = name.split(':');
        let mut artifact_contract_name =
            split.next().ok_or_else(|| eyre::Error::msg("no contract name provided"))?;
        if let Some(new_name) = split.next() {
            artifact_contract_name = new_name;
        };

        if artifact_contract_name == contract.name {
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
                .ok_or_else(|| eyre::Error::msg(format!("abi not found for {}", contract.name)))?,
            artifact.bin.ok_or_else(|| {
                eyre::Error::msg(format!("bytecode not found for {}", contract.name))
            })?,
            artifact.bin_runtime.ok_or_else(|| {
                eyre::Error::msg(format!("bytecode not found for {}", contract.name))
            })?,
        ),
        None => {
            eyre::bail!("could not find artifact")
        }
    })
}

/// Find using src/ContractSource.sol:ContractName
// TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
// path?
fn get_artifact_from_path(
    project: &Project,
    path: String,
    name: String,
) -> eyre::Result<(Abi, BytecodeObject, BytecodeObject)> {
    // Get sources from the requested location
    let abs_path = dunce::canonicalize(PathBuf::from(path))?;
    let mut sources = Sources::new();
    sources.insert(abs_path.clone(), Source::read(&abs_path)?);

    // Get artifact from the contract name and sources
    let mut config = SolFilesCache::builder().insert_files(sources.clone(), None)?;
    config.files.entry(abs_path).and_modify(|f| f.artifacts = vec![name.clone()]);

    let artifacts = config
        .read_artifacts::<MinimalCombinedArtifacts>(project.artifacts_path())?
        .into_values()
        .collect::<Vec<_>>();

    if artifacts.is_empty() {
        eyre::bail!("could not find artifact")
    } else if artifacts.len() > 1 {
        eyre::bail!("duplicate contract name in the same source file")
    }
    let artifact = artifacts[0].clone();

    Ok((
        artifact.abi.ok_or_else(|| eyre::Error::msg(format!("abi not found for {}", name)))?,
        artifact.bin.ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {}", name)))?,
        artifact
            .bin_runtime
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {}", name)))?,
    ))
}
