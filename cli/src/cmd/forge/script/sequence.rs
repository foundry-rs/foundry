use super::{NestedValue, ScriptResult};
use crate::cmd::forge::{init::get_commit_hash, script::verify::VerifyBundle};
use cast::{executor::inspector::DEFAULT_CREATE2_DEPLOYER, CallKind};
use ethers::{
    abi::{Abi, Address},
    prelude::{artifacts::Libraries, ArtifactId, NameOrAddress, TransactionReceipt, TxHash},
    types::transaction::eip2718::TypedTransaction,
};
use eyre::ContextCompat;
use forge::trace::CallTraceDecoder;
use foundry_common::{fs, SELECTOR_LEN};
use foundry_config::Config;

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::BufWriter,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::trace;
use yansi::Paint;

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
    pub commit: Option<String>,
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
        let commit = get_commit_hash(&config.__root.0);

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
            commit,
        })
    }

    /// Loads The sequence for the corresponding json file
    pub fn load(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
    ) -> eyre::Result<Self> {
        let path = ScriptSequence::get_path(&config.broadcast, sig, target, chain_id)?;
        Ok(ethers::solc::utils::read_json_file(path)?)
    }

    /// Saves the transactions as files
    pub fn save(&mut self) -> eyre::Result<()> {
        if !self.transactions.is_empty() {
            self.timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let path = self.path.to_string_lossy();
            //../run-latest.json
            serde_json::to_writer_pretty(BufWriter::new(fs::create_file(&self.path)?), &self)?;
            //../run-[timestamp].json
            serde_json::to_writer_pretty(
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

    /// Sorts all receipts with ascending transaction index
    pub fn sort_receipts(&mut self) {
        self.receipts.sort_unstable()
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
        out.push(chain_id.to_string());

        fs::create_dir_all(&out)?;

        // TODO: ideally we want the name of the function here if sig is calldata
        let filename = sig_to_file_name(sig);

        out.push(format!("{filename}-latest.json"));
        Ok(out)
    }

    /// Given the broadcast log, it matches transactions with receipts, and tries to verify any
    /// created contract on etherscan.
    pub async fn verify_contracts(&mut self, verify: VerifyBundle, chain: u64) -> eyre::Result<()> {
        trace!(?chain, "verifying {} contracts", verify.known_contracts.len());
        if verify.etherscan_key.is_some() {
            let mut future_verifications = vec![];
            let mut unverifiable_contracts = vec![];

            // Make sure the receipts have the right order first.
            self.sort_receipts();

            for (receipt, tx) in self.receipts.iter_mut().zip(self.transactions.iter()) {
                // create2 hash offset
                let mut offset = 0;

                if tx.is_create2() {
                    receipt.contract_address = tx.contract_address;
                    offset = 32;
                }

                // Verify contract created directly from the transaction
                if let (Some(address), Some(data)) =
                    (receipt.contract_address, tx.typed_tx().data())
                {
                    match verify.get_verify_args(address, offset, &data.0, &self.libraries) {
                        Some(verify) => future_verifications.push(verify.run()),
                        None => unverifiable_contracts.push(address),
                    };
                }

                // Verify potential contracts created during the transaction execution
                for AdditionalContract { address, init_code, .. } in &tx.additional_contracts {
                    match verify.get_verify_args(*address, 0, init_code, &self.libraries) {
                        Some(verify) => future_verifications.push(verify.run()),
                        None => unverifiable_contracts.push(*address),
                    };
                }
            }

            self.check_unverified(unverifiable_contracts, verify);

            let num_verifications = future_verifications.len();
            println!("##\nStart verification for ({num_verifications}) contracts",);
            for verification in future_verifications {
                verification.await?;
            }

            println!("All ({num_verifications}) contracts were verified!");
        }

        Ok(())
    }

    /// Let the user know if there are any contracts which can not be verified. Also, present some
    /// hints on potential causes.
    fn check_unverified(&self, unverifiable_contracts: Vec<Address>, verify: VerifyBundle) {
        if !unverifiable_contracts.is_empty() {
            println!(
                "\n{}",
                Paint::yellow(format!(
                    "We haven't found any matching bytecode for the following contracts: {:?}.\n\n{}",
                    unverifiable_contracts,
                    "This may occur when resuming a verification, but the underlying source code or compiler version has changed."
                ))
                .bold(),
            );

            if let Some(commit) = &self.commit {
                let current_commit = verify
                    .project_paths
                    .root
                    .map(|root| get_commit_hash(&root).unwrap_or_default())
                    .unwrap_or_default();

                if &current_commit != commit {
                    println!("\tScript was broadcasted on commit `{commit}`, but we are at `{current_commit}`.");
                }
            }
        }
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
pub struct AdditionalContract {
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    pub address: Address,
    #[serde(with = "hex")]
    pub init_code: Vec<u8>,
}

#[derive(Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithMetadata {
    pub hash: Option<TxHash>,
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    #[serde(default = "default_string")]
    pub contract_name: Option<String>,
    #[serde(default = "default_address")]
    pub contract_address: Option<Address>,
    #[serde(default = "default_string")]
    pub function: Option<String>,
    #[serde(default = "default_vec_of_strings")]
    pub arguments: Option<Vec<String>>,
    pub transaction: TypedTransaction,
    pub additional_contracts: Vec<AdditionalContract>,
}

fn default_string() -> Option<String> {
    Some("".to_owned())
}

fn default_address() -> Option<Address> {
    Some(Address::from_low_u64_be(0))
}

fn default_vec_of_strings() -> Option<Vec<String>> {
    Some(vec![])
}

impl TransactionWithMetadata {
    pub fn from_typed_transaction(transaction: TypedTransaction) -> Self {
        Self { transaction, ..Default::default() }
    }

    pub fn new(
        transaction: TypedTransaction,
        result: &ScriptResult,
        local_contracts: &BTreeMap<Address, (String, &Abi)>,
        decoder: &CallTraceDecoder,
        additional_contracts: Vec<AdditionalContract>,
    ) -> eyre::Result<Self> {
        let mut metadata = Self { transaction, ..Default::default() };

        // Specify if any contract was directly created with this transaction
        if let Some(NameOrAddress::Address(to)) = metadata.transaction.to().cloned() {
            if to == DEFAULT_CREATE2_DEPLOYER {
                metadata.set_create(true, Address::from_slice(&result.returned), local_contracts)
            } else {
                metadata.set_call(to, local_contracts, decoder)?;
            }
        } else if metadata.transaction.to().is_none() {
            metadata.set_create(
                false,
                result.address.expect("There should be a contract address."),
                local_contracts,
            );
        }

        // Add the additional contracts created in this transaction, so we can verify them later.
        if let Some(tx_address) = metadata.contract_address {
            metadata.additional_contracts = additional_contracts
                .into_iter()
                .filter_map(|contract| {
                    // Filter out the transaction contract repeated init_code.
                    if contract.address != tx_address {
                        Some(contract)
                    } else {
                        None
                    }
                })
                .collect();
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
            self.opcode = CallKind::Create2;
        } else {
            self.opcode = CallKind::Create;
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
        self.opcode = CallKind::Create;

        if let Some(data) = self.transaction.data() {
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
        self.transaction = tx;
    }

    pub fn change_type(&mut self, is_legacy: bool) {
        self.transaction = if is_legacy {
            TypedTransaction::Legacy(self.transaction.clone().into())
        } else {
            TypedTransaction::Eip1559(self.transaction.clone().into())
        };
    }

    pub fn typed_tx(&self) -> &TypedTransaction {
        &self.transaction
    }

    pub fn typed_tx_mut(&mut self) -> &mut TypedTransaction {
        &mut self.transaction
    }

    pub fn is_create2(&self) -> bool {
        self.opcode == CallKind::Create2
    }
}

/// Converts the `sig` argument into the corresponding file path.
///
/// This accepts either the signature of the function or the raw calldata

fn sig_to_file_name(sig: &str) -> String {
    if let Some((name, _)) = sig.split_once('(') {
        // strip until call argument parenthesis
        return name.to_string()
    }
    // assume calldata if `sig` is hex
    if let Ok(calldata) = hex::decode(sig) {
        // in which case we return the function signature
        return hex::encode(&calldata[..SELECTOR_LEN])
    }

    // return sig as is
    sig.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_convert_sig() {
        assert_eq!(sig_to_file_name("run()").as_str(), "run");
        assert_eq!(
            sig_to_file_name(
                "522bb704000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfFFb92266"
            )
            .as_str(),
            "522bb704"
        );
    }
}
