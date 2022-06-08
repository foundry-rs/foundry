use crate::{opts::forge::ContractInfo, suggestions};
use cast::executor::inspector::DEFAULT_CREATE2_DEPLOYER;
use clap::Parser;
use ethers::{
    abi::{Abi, Address},
    core::types::Chain,
    prelude::{artifacts::Libraries, ArtifactId, NameOrAddress, TransactionReceipt, TxHash},
    solc::{
        artifacts::{
            CompactBytecode, CompactContractBytecode, CompactDeployedBytecode, ContractBytecodeSome,
        },
        cache::{CacheEntry, SolFilesCache},
        Project,
    },
    types::transaction::eip2718::TypedTransaction,
};
use foundry_config::Config;
use foundry_utils::Retry;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io::BufWriter,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use yansi::Paint;

use super::forge::{
    build::ProjectPathsArgs,
    create::RETRY_VERIFY_ON_CREATE,
    verify::{self},
};

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
        validator = u32_validator(1, 10),
        value_name = "RETRIES"
    )]
    pub retries: u32,

    #[clap(
        long,
        help = "Optional timeout to apply inbetween attempts in seconds.",
        validator = u32_validator(0, 30),
        value_name = "DELAY"
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
    deployed_code: bool,
) -> BTreeMap<ArtifactId, (Abi, Vec<u8>)> {
    contracts
        .iter()
        .map(|(id, c)| {
            let bytecode = if deployed_code {
                c.deployed_bytecode.clone().into_bytes().expect("not bytecode").to_vec()
            } else {
                c.bytecode.clone().object.into_bytes().expect("not bytecode").to_vec()
            };

            (id.clone(), (c.abi.clone(), bytecode))
        })
        .collect()
}

/// Data struct to help `ScriptSequence` verify contracts on `etherscan`.
pub struct VerifyBundle {
    pub num_of_optimizations: Option<usize>,
    pub known_contracts: BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    pub etherscan_key: Option<String>,
    pub project_paths: ProjectPathsArgs,
}

impl VerifyBundle {
    pub fn new(
        project: &Project,
        config: &Config,
        known_contracts: BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    ) -> Self {
        let num_of_optimizations =
            if config.optimizer { Some(config.optimizer_runs) } else { None };

        let config_path = config.get_config_path();

        let project_paths = ProjectPathsArgs {
            root: Some(project.paths.root.clone()),
            contracts: Some(project.paths.sources.clone()),
            remappings: project.paths.remappings.clone(),
            remappings_env: None,
            cache_path: Some(project.paths.cache.clone()),
            lib_paths: project.paths.libraries.clone(),
            hardhat: config.profile == Config::HARDHAT_PROFILE,
            config_path: if config_path.exists() { Some(config_path) } else { None },
        };

        VerifyBundle {
            num_of_optimizations,
            known_contracts,
            etherscan_key: config.etherscan_api_key.clone(),
            project_paths,
        }
    }
}
/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
#[derive(Deserialize, Serialize, Clone)]
pub struct ScriptSequence {
    pub transactions: VecDeque<TypedTransaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub pending: Vec<TxHash>,
    pub create2_contracts: Vec<Address>,
    pub path: PathBuf,
    pub timestamp: u64,
    pub libraries: Vec<String>,
}

impl ScriptSequence {
    pub fn new(
        transactions: VecDeque<TypedTransaction>,
        sig: &str,
        target: &ArtifactId,
        config: &Config,
        chain_id: u64,
    ) -> eyre::Result<Self> {
        let path = ScriptSequence::get_path(&config.broadcast, sig, target, chain_id)?;

        Ok(ScriptSequence {
            transactions,
            receipts: vec![],
            pending: vec![],
            create2_contracts: vec![],
            path,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Wrong system time.")
                .as_secs(),
            libraries: vec![],
        })
    }

    pub fn load(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
    ) -> eyre::Result<Self> {
        let file = std::fs::read_to_string(ScriptSequence::get_path(
            &config.broadcast,
            sig,
            target,
            chain_id,
        )?)?;
        serde_json::from_str(&file).map_err(|e| e.into())
    }

    pub fn save(&mut self) -> eyre::Result<()> {
        if !self.transactions.is_empty() {
            self.timestamp =
                SystemTime::now().duration_since(UNIX_EPOCH).expect("Wrong system time.").as_secs();

            let path = self.path.to_str().expect("Invalid path.");

            //../run-latest.json
            serde_json::to_writer(BufWriter::new(std::fs::File::create(path)?), &self)?;
            //../run-timestamp.json
            serde_json::to_writer(
                BufWriter::new(std::fs::File::create(
                    &path.replace("latest.json", &format!("{}.json", self.timestamp)),
                )?),
                &self,
            )?;

            println!(
                "\nTransactions saved to: {}\n",
                self.path.to_str().expect(
                    "Couldn't convert path to string. Transactions were written to file though."
                )
            );
        }

        Ok(())
    }

    pub fn add_receipt(&mut self, receipt: TransactionReceipt) {
        self.receipts.push(receipt);
    }

    pub fn sort_receipts(&mut self) {
        self.receipts.sort_by(|a, b| {
            let ablock = a.block_number.unwrap();
            let bblock = b.block_number.unwrap();
            if ablock == bblock {
                a.transaction_index.cmp(&b.transaction_index)
            } else {
                ablock.cmp(&bblock)
            }
        });
    }

    pub fn add_pending(&mut self, tx_hash: TxHash) {
        if !self.pending.contains(&tx_hash) {
            self.pending.push(tx_hash);
        }
    }

    pub fn remove_pending(&mut self, tx_hash: TxHash) {
        self.pending.retain(|element| element != &tx_hash);
    }

    pub fn add_create2(&mut self, address: Address) {
        self.create2_contracts.push(address);
    }

    pub fn add_libraries(&mut self, libraries: Libraries) {
        self.libraries = {
            let mut str_libs = vec![];
            for (file, libs) in libraries.libs {
                for (name, address) in libs {
                    str_libs.push(format!("{}:{}:{}", file.to_string_lossy(), name, address));
                }
            }
            str_libs
        };
    }

    /// Saves to ./broadcast/contract_filename/sig[-timestamp].json
    pub fn get_path(
        out: &Path,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
    ) -> eyre::Result<PathBuf> {
        let mut out = out.to_path_buf();

        let target_fname = target.source.file_name().expect("No file name");
        out.push(target_fname);
        out.push(format!("{chain_id}"));

        std::fs::create_dir_all(out.clone())?;

        let filename = sig.split_once('(').expect("Sig is invalid").0.to_owned();
        out.push(format!("{filename}-latest.json"));
        Ok(out)
    }

    /// Given the broadcast log, it matches transactions with receipts, and tries to verify any
    /// created contract on etherscan.
    pub async fn verify_contracts(&mut self, verify: VerifyBundle, chain: u64) -> eyre::Result<()> {
        if let Some(etherscan_key) = &verify.etherscan_key {
            let mut future_verifications = vec![];
            let mut create2 = self.create2_contracts.clone().into_iter();

            // Make sure the receipts have the right order first.
            self.sort_receipts();

            for (receipt, tx) in self.receipts.iter_mut().zip(self.transactions.iter()) {
                let mut create2_offset = 0;

                // CREATE2 contract addresses do not come in the receipt.
                if let Some(&NameOrAddress::Address(to)) = tx.to() {
                    if to == DEFAULT_CREATE2_DEPLOYER {
                        receipt.contract_address = create2.next();
                        create2_offset = 32;
                    }
                }

                if let (Some(contract_address), Some(data)) = (receipt.contract_address, tx.data())
                {
                    for (artifact, (_contract, bytecode)) in &verify.known_contracts {
                        // If it's a CREATE2, the tx.data comes with a 32-byte salt in the beginning
                        // of the transaction
                        if data.0.split_at(create2_offset).1.starts_with(bytecode) {
                            let constructor_args =
                                data.0.split_at(create2_offset + bytecode.len()).1.to_vec();

                            let contract = ContractInfo {
                                path: Some(
                                    artifact
                                        .source
                                        .to_str()
                                        .expect("There should be an artifact.")
                                        .to_string(),
                                ),
                                name: artifact.name.clone(),
                            };

                            // We strip the build metadadata information, since it can lead to
                            // etherscan not identifying it correctly. eg:
                            // `v0.8.10+commit.fc410830.Linux.gcc` != `v0.8.10+commit.fc410830`
                            let version = Version::new(
                                artifact.version.major,
                                artifact.version.minor,
                                artifact.version.patch,
                            );

                            let verify = verify::VerifyArgs {
                                address: contract_address,
                                contract,
                                compiler_version: Some(version.to_string()),
                                constructor_args: Some(hex::encode(&constructor_args)),
                                num_of_optimizations: verify.num_of_optimizations,
                                chain: chain.into(),
                                etherscan_key: etherscan_key.clone(),
                                project_paths: verify.project_paths.clone(),
                                flatten: false,
                                force: false,
                                watch: true,
                                retry: RETRY_VERIFY_ON_CREATE,
                                libraries: self.libraries.clone(),
                            };

                            future_verifications.push(verify.run());
                        }
                    }
                }
            }

            println!("##\nStart Contract Verification");
            for verification in future_verifications {
                verification.await?;
            }
        }

        Ok(())
    }
}

impl Drop for ScriptSequence {
    fn drop(&mut self) {
        self.sort_receipts();
        self.save().expect("not able to save deployment sequence");
    }
}

#[macro_export]
macro_rules! init_progress {
    ($local:expr, $label:expr) => {{
        let pb = ProgressBar::new($local.len() as u64);
        let mut template =
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ".to_string();
        template += $label;
        template += " ({eta})";
        pb.set_style(
            ProgressStyle::with_template(&template)
                .unwrap()
                .with_key("eta", |state| format!("{:.1}s", state.eta().as_secs_f64()))
                .progress_chars("#>-"),
        );
        pb
    }};
}

#[macro_export]
macro_rules! update_progress {
    ($pb:ident, $index:expr) => {
        $pb.set_position(($index + 1) as u64);
    };
}

/// True if the network calculates gas costs differently.
pub fn has_different_gas_calc(chain: u64) -> bool {
    matches!(
        Chain::try_from(chain).unwrap_or(Chain::Mainnet),
        Chain::Arbitrum | Chain::ArbitrumTestnet | Chain::Optimism | Chain::OptimismKovan
    )
}
