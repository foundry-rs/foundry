use crate::{opts::forge::ContractInfo, suggestions};
use ethers::{
    abi::Abi,
    prelude::{
        artifacts::{CompactBytecode, CompactDeployedBytecode},
        ArtifactId, TransactionReceipt,
    },
    types::transaction::eip2718::TypedTransaction,
    solc::{
        artifacts::CompactContractBytecode, cache::SolFilesCache, Project, ProjectCompileOutput,
    }
};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io::Write, path::PathBuf};

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

#[derive(Deserialize, Serialize, Clone)]
pub struct ScriptSequence {
    pub index: u32,
    pub transactions: VecDeque<TypedTransaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub path: PathBuf,
}

impl ScriptSequence {
    pub fn new(
        transactions: VecDeque<TypedTransaction>,
        sig: &String,
        target: &ArtifactId,
        out: &PathBuf,
    ) -> eyre::Result<Self> {
        Ok(ScriptSequence {
            index: 0,
            transactions,
            receipts: vec![],
            path: ScriptSequence::get_path(sig, target, out)?,
        })
    }

    pub fn load(sig: &String, target: &ArtifactId, out: &PathBuf) -> eyre::Result<Self> {
        let file = std::fs::read_to_string(ScriptSequence::get_path(sig, target, out)?)?;
        serde_json::from_str(&file).map_err(|e| e.into())
    }

    pub fn save(&self) -> eyre::Result<()> {
        let tx_json = serde_json::to_string_pretty(&self).expect("Bad serializing");
        println!("\nGenerated Transactions:\n\n{}", tx_json);

        let mut file = std::fs::File::create(&self.path)?;
        file.write_all(tx_json.as_bytes())?;

        println!(
            "\nTransactions written to: {}\n",
            self.path.to_str().expect(
                "Couldn't convert path to string. Transactions were written to file though."
            )
        );

        Ok(())
    }

    pub fn add_receipt(&mut self, receipt: TransactionReceipt) {
        self.receipts.push(receipt);
    }

    pub fn get_path(sig: &String, target: &ArtifactId, out: &PathBuf) -> eyre::Result<PathBuf> {
        let mut out = out.clone();
        let target_fname = target.source.file_name().expect("No file name");
        out.push(target_fname);
        out.push("scripted_transactions");
        std::fs::create_dir_all(out.clone())?;
        out.push(sig.to_owned() + ".json");
        Ok(out)
    }
}
