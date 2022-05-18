use crate::{opts::forge::ContractInfo, suggestions};
use clap::Parser;
use ethers::{
    abi::Abi,
    prelude::{
        artifacts::{CompactBytecode, CompactDeployedBytecode, ContractBytecodeSome},
        cache::CacheEntry,
        ArtifactId, TransactionReceipt,
    },
    solc::{
        artifacts::{CompactBytecode, CompactContractBytecode, CompactDeployedBytecode},
        cache::SolFilesCache,
        Project,
    },
    types::transaction::eip2718::TypedTransaction,
};
use foundry_utils::Retry;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io::BufWriter,
    path::{Path, PathBuf},
};
use yansi::Paint;

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
    contract: ContractInfo,
) -> eyre::Result<(Abi, CompactBytecode, CompactDeployedBytecode)> {
    let cache = SolFilesCache::read_joined(&project.paths)?;
    let contract_path = match contract.path {
        Some(path) => dunce::canonicalize(PathBuf::from(path))?,
        None => get_cached_entry_by_name(&cache, &contract.name)?.0,
    };

    let artifact: CompactContractBytecode = cache.read_artifact(contract_path, &contract.name)?;

    Ok((
        artifact
            .abi
            .ok_or_else(|| eyre::Error::msg(format!("abi not found for {}", contract.name)))?,
        artifact
            .bytecode
            .ok_or_else(|| eyre::Error::msg(format!("bytecode not found for {}", contract.name)))?,
        artifact.deployed_bytecode.ok_or_else(|| {
            eyre::Error::msg(format!("deployed bytecode not found for {}", contract.name))
        })?,
    ))
}

/// Helper function for finding a contract by ContractName
// TODO: Is there a better / more ergonomic way to get the artifacts given a project and a
// contract name?
pub fn get_cached_entry_by_name(
    cache: &SolFilesCache,
    name: &str,
) -> eyre::Result<(PathBuf, CacheEntry)> {
    let mut cached_entry = None;
    let mut alternatives = Vec::new();

    for (abs_path, entry) in cache.files.iter() {
        for (artifact_name, _) in entry.artifacts.iter() {
            if artifact_name == name {
                if cached_entry.is_some() {
                    eyre::bail!(
                        "contract with duplicate name `{}`. please pass the path instead",
                        name
                    )
                }
                cached_entry = Some((abs_path.to_owned(), entry.to_owned()));
            } else {
                alternatives.push(artifact_name);
            }
        }
    }

    if let Some(entry) = cached_entry {
        return Ok(entry)
    }

    let mut err = format!("could not find artifact: `{}`", name);
    if let Some(suggestion) = suggestions::did_you_mean(name, &alternatives).pop() {
        err = format!(
            r#"{}

        Did you mean `{}`?"#,
            err, suggestion
        );
    }
    eyre::bail!(err)
}

/// A type that keeps track of attempts
#[derive(Debug, Clone, Parser)]
pub struct RetryArgs {
    #[clap(
        long,
        help = "Number of attempts for retrying",
        default_value = "1",
        validator = u32_validator(1, 10)
    )]
    pub retries: u32,

    #[clap(
        long,
        help = "Optional timeout to apply inbetween attempts in seconds.",
        validator = u32_validator(0, 30)
    )]
    pub delay: Option<u32>,
}

fn u32_validator(min: u32, max: u32) -> impl FnMut(&str) -> eyre::Result<()> {
    move |v: &str| -> eyre::Result<()> {
        let v = v.parse::<u32>()?;
        if v >= min && v <= max {
            Ok(())
        } else {
            Err(eyre::eyre!("Expected between {} and {} inclusive.", min, max))
        }
    }
}

impl From<RetryArgs> for Retry {
    fn from(r: RetryArgs) -> Self {
        Retry::new(r.retries, r.delay)
    }
}

pub fn needs_setup(abi: &Abi) -> bool {
    let setup_fns: Vec<_> =
        abi.functions().filter(|func| func.name.to_lowercase() == "setup").collect();

    for setup_fn in setup_fns.iter() {
        if setup_fn.name != "setUp" {
            println!(
                "{} Found invalid setup function \"{}\" did you mean \"setUp()\"?",
                Paint::yellow("Warning:").bold(),
                setup_fn.signature()
            );
        }
    }

    setup_fns.len() == 1 && setup_fns[0].name == "setUp"
}

pub fn unwrap_contracts(
    contracts: &BTreeMap<ArtifactId, ContractBytecodeSome>,
) -> BTreeMap<ArtifactId, (Abi, Vec<u8>)> {
    contracts
        .iter()
        .map(|(id, c)| {
            (
                id.clone(),
                (
                    c.abi.clone(),
                    c.deployed_bytecode.clone().into_bytes().expect("not bytecode").to_vec(),
                ),
            )
        })
        .collect()
}

/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
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
        sig: &str,
        target: &ArtifactId,
        out: &Path,
    ) -> eyre::Result<Self> {
        Ok(ScriptSequence {
            index: 0,
            transactions,
            receipts: vec![],
            path: ScriptSequence::get_path(sig, target, out)?,
        })
    }

    pub fn load(sig: &str, target: &ArtifactId, out: &Path) -> eyre::Result<Self> {
        let file = std::fs::read_to_string(ScriptSequence::get_path(sig, target, out)?)?;
        serde_json::from_str(&file).map_err(|e| e.into())
    }

    pub fn save(&self) -> eyre::Result<()> {
        serde_json::to_writer(BufWriter::new(std::fs::File::create(&self.path)?), &self)?;

        println!(
            "\nTransactions saved to: {}\n",
            self.path.to_str().expect(
                "Couldn't convert path to string. Transactions were written to file though."
            )
        );

        Ok(())
    }

    pub fn add_receipt(&mut self, receipt: TransactionReceipt) {
        self.receipts.push(receipt);
    }

    /// Saves to ./out/_CONTRACT_FILE_NAME_/scripted_transactions/_SIG_().json
    pub fn get_path(sig: &str, target: &ArtifactId, out: &Path) -> eyre::Result<PathBuf> {
        let mut out = out.to_path_buf();
        let target_fname = target.source.file_name().expect("No file name");
        out.push(target_fname);
        out.push("scripted_transactions");
        std::fs::create_dir_all(out.clone())?;
        out.push(sig.to_owned() + ".json");
        Ok(out)
    }
}

impl Drop for ScriptSequence {
    fn drop(&mut self) {
        self.save().expect("not able to save deployment sequence");
    }
}
