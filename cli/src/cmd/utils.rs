use crate::{opts::forge::ContractInfo, suggestions};
use ethers::{
    abi::Abi,
    prelude::artifacts::{CompactBytecode, CompactDeployedBytecode},
    solc::{
        artifacts::CompactContractBytecode, cache::SolFilesCache, Project, ProjectCompileOutput,
    },
};
use std::path::PathBuf;

/// Common trait for all cli commands
pub trait Cmd: clap::Parser + Sized {
    type Output;
    fn run(self) -> eyre::Result<Self::Output>;
}

/// Given a project and its compiled artifacts, proceeds to return the ABI, Bytecode and
/// Runtime Bytecode of the given contract.
#[track_caller]
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
    let mut contract_artifact = None;
    let mut alternatives = Vec::new();

    for (artifact_id, artifact) in compiled.into_artifacts() {
        if artifact_id.name == contract.name {
            if contract_artifact.is_some() {
                eyre::bail!(
                    "contract with duplicate name `{}`. please pass the path instead",
                    contract.name
                )
            }
            contract_artifact = Some(artifact);
        } else {
            alternatives.push(artifact_id.name);
        }
    }

    if let Some(artifact) = contract_artifact {
        let abi = artifact
            .abi
            .map(Into::into)
            .ok_or_else(|| eyre::eyre!("abi not found for {}", contract.name))?;

        let code = artifact
            .bytecode
            .ok_or_else(|| eyre::eyre!("bytecode not found for {}", contract.name))?;

        let deployed_code = artifact
            .deployed_bytecode
            .ok_or_else(|| eyre::eyre!("bytecode not found for {}", contract.name))?;
        return Ok((abi, code, deployed_code))
    }

    let mut err = format!("could not find artifact: `{}`", contract.name);
    if let Some(suggestion) = suggestions::did_you_mean(&contract.name, &alternatives).pop() {
        err = format!(
            r#"{}

        Did you mean `{}`?"#,
            err, suggestion
        );
    }
    eyre::bail!(err)
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
            .ok_or_else(|| eyre::Error::msg(format!("abi not found for {contract_name}")))?,
        artifact
            .bytecode
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {contract_name}")))?,
        artifact
            .deployed_bytecode
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {contract_name}")))?,
    ))
}
