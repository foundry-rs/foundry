use super::{NestedValue, ScriptResult, VerifyBundle};
use crate::cmd::forge::{create::RETRY_VERIFY_ON_CREATE, verify};
use cast::executor::inspector::DEFAULT_CREATE2_DEPLOYER;
use ethers::{
    abi::{Abi, Address},
    prelude::{artifacts::Libraries, ArtifactId, NameOrAddress, TransactionReceipt, TxHash},
    solc::info::ContractInfo,
    types::transaction::eip2718::TypedTransaction,
};
use eyre::ContextCompat;
use forge::trace::CallTraceDecoder;
use foundry_common::fs;
use foundry_config::Config;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::BufWriter,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
#[derive(Deserialize, Serialize, Clone)]
pub struct ScriptSequence {
    pub transactions: VecDeque<TransactionWithMetadata>,
    pub receipts: Vec<TransactionReceipt>,
    pub libraries: Vec<String>,
    pub pending: Vec<TxHash>,
    pub path: PathBuf,
    pub returns: HashMap<String, NestedValue>,
    pub timestamp: u64,
}

impl ScriptSequence {
    pub fn new(
        transactions: VecDeque<TransactionWithMetadata>,
        returns: HashMap<String, NestedValue>,
        sig: &str,
        target: &ArtifactId,
        config: &Config,
        chain_id: u64,
    ) -> eyre::Result<Self> {
        let path = ScriptSequence::get_path(&config.broadcast, sig, target, chain_id)?;

        Ok(ScriptSequence {
            transactions,
            returns,
            receipts: vec![],
            pending: vec![],
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
        let file = fs::read_to_string(ScriptSequence::get_path(
            &config.broadcast,
            sig,
            target,
            chain_id,
        )?)?;
        serde_json::from_str(&file).map_err(|e| e.into())
    }

    pub fn save(&mut self) -> eyre::Result<()> {
        if !self.transactions.is_empty() {
            self.timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let path = self.path.to_string_lossy();
            //../run-latest.json
            serde_json::to_writer(BufWriter::new(fs::create_file(&self.path)?), &self)?;
            //../run-[timestamp].json
            serde_json::to_writer(
                BufWriter::new(fs::create_file(
                    path.replace("latest.json", &format!("{}.json", self.timestamp)),
                )?),
                &self,
            )?;

            println!("\nTransactions saved to: {path}\n");
        }

        Ok(())
    }

    pub fn add_receipt(&mut self, receipt: TransactionReceipt) {
        self.receipts.push(receipt);
    }

    pub fn sort_receipts(&mut self) {
        self.receipts.sort_by(|a, b| {
            // Safe since `block_number` is present in receipts.
            let ablock = a.block_number.unwrap();
            let bblock = b.block_number.unwrap();
            if ablock == bblock {
                a.transaction_index.cmp(&b.transaction_index)
            } else {
                ablock.cmp(&bblock)
            }
        });
    }

    pub fn add_pending(&mut self, index: usize, tx_hash: TxHash) {
        if !self.pending.contains(&tx_hash) {
            self.transactions[index].hash = Some(tx_hash);
            self.pending.push(tx_hash);
        }
    }

    pub fn remove_pending(&mut self, tx_hash: TxHash) {
        self.pending.retain(|element| element != &tx_hash);
    }

    pub fn add_libraries(&mut self, libraries: Libraries) {
        self.libraries = libraries
            .libs
            .iter()
            .flat_map(|(file, libs)| {
                libs.iter().map(|(name, address)| {
                    format!("{}:{}:{}", file.to_string_lossy(), name, address)
                })
            })
            .collect();
    }

    /// Saves to ./broadcast/contract_filename/sig[-timestamp].json
    pub fn get_path(
        out: &Path,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
    ) -> eyre::Result<PathBuf> {
        let mut out = out.to_path_buf();

        let target_fname = target.source.file_name().wrap_err("No filename.")?;
        out.push(target_fname);
        out.push(format!("{chain_id}"));

        fs::create_dir_all(&out)?;

        let filename = sig.split_once('(').wrap_err("Sig is invalid.")?.0.to_owned();
        out.push(format!("{filename}-latest.json"));
        Ok(out)
    }

    /// Given the broadcast log, it matches transactions with receipts, and tries to verify any
    /// created contract on etherscan.
    pub async fn verify_contracts(&mut self, verify: VerifyBundle, chain: u64) -> eyre::Result<()> {
        if let Some(etherscan_key) = &verify.etherscan_key {
            let mut future_verifications = vec![];

            // Make sure the receipts have the right order first.
            self.sort_receipts();

            for (receipt, tx) in self.receipts.iter_mut().zip(self.transactions.iter()) {
                let mut create2_offset = 0;

                if tx.is_create2() {
                    receipt.contract_address = tx.contract_address;
                    create2_offset = 32;
                }

                if let (Some(contract_address), Some(data)) =
                    (receipt.contract_address, tx.typed_tx().data())
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

    /// Returns the list of the transactions without the metadata.
    pub fn typed_transactions(&self) -> Vec<&TypedTransaction> {
        self.transactions.iter().map(|tx| tx.typed_tx()).collect()
    }
}

impl Drop for ScriptSequence {
    fn drop(&mut self) {
        self.sort_receipts();
        self.save().expect("not able to save deployment sequence");
    }
}

#[derive(Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithMetadata {
    pub hash: Option<TxHash>,
    #[serde(rename = "type")]
    pub opcode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<String>>,
    pub tx: TypedTransaction,
}

impl TransactionWithMetadata {
    pub fn new(
        tx: TypedTransaction,
        result: &ScriptResult,
        local_contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
    ) -> eyre::Result<Self> {
        let mut metadata = Self { tx, ..Default::default() };

        if let Some(NameOrAddress::Address(to)) = metadata.tx.to().cloned() {
            if to == DEFAULT_CREATE2_DEPLOYER {
                metadata.set_create(true, Address::from_slice(&result.returned), local_contracts)
            } else {
                metadata.set_call(to, local_contracts, decoder)?;
            }
        } else if metadata.tx.to().is_none() {
            metadata.set_create(
                false,
                result.address.expect("There should be a contract address."),
                local_contracts,
            );
        }
        Ok(metadata)
    }

    fn set_create(
        &mut self,
        is_create2: bool,
        address: Address,
        contracts: &BTreeMap<Address, (String, &Abi)>,
    ) {
        if is_create2 {
            self.opcode = "CREATE2".to_string();
        } else {
            self.opcode = "CREATE".to_string();
        }

        self.contract_name = contracts.get(&address).map(|(name, _)| name.clone());
        self.contract_address = Some(address);
    }

    fn set_call(
        &mut self,
        target: Address,
        local_contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
    ) -> eyre::Result<()> {
        self.opcode = "CALL".to_string();

        if let Some(data) = self.tx.data() {
            if data.0.len() >= 4 {
                if let Some((contract_name, abi)) = local_contracts.get(&target) {
                    // This CALL is made to a local contract.

                    self.contract_name = Some(contract_name.clone());
                    if let Some(function) =
                        abi.functions().find(|function| function.short_signature() == data.0[0..4])
                    {
                        self.function = Some(function.signature());
                        self.arguments =
                            Some(function.decode_input(&data.0[4..]).map(|tokens| {
                                tokens.iter().map(|token| format!("{token}")).collect()
                            })?);
                    }
                } else {
                    // This CALL is made to an external contract. We can only decode it, if it has
                    // been verified and identified by etherscan.

                    if let Some(Some(function)) =
                        decoder.functions.get(&data.0[0..4]).map(|functions| functions.first())
                    {
                        self.contract_name = decoder.contracts.get(&target).cloned();

                        self.function = Some(function.signature());
                        self.arguments =
                            Some(function.decode_input(&data.0[4..]).map(|tokens| {
                                tokens.iter().map(|token| format!("{token}")).collect()
                            })?);
                    }
                }
                self.contract_address = Some(target);
            }
        }
        Ok(())
    }

    pub fn set_tx(&mut self, tx: TypedTransaction) {
        self.tx = tx;
    }

    pub fn change_type(&mut self, is_legacy: bool) {
        self.tx = if is_legacy {
            TypedTransaction::Legacy(self.tx.clone().into())
        } else {
            TypedTransaction::Eip1559(self.tx.clone().into())
        };
    }

    pub fn typed_tx(&self) -> &TypedTransaction {
        &self.tx
    }

    pub fn typed_tx_mut(&mut self) -> &mut TypedTransaction {
        &mut self.tx
    }

    pub fn is_create2(&self) -> bool {
        self.opcode == "CREATE2"
    }
}
