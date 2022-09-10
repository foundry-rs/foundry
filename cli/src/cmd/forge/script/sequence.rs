use super::{NestedValue, ScriptResult};
use crate::cmd::forge::{init::get_commit_hash, script::verify::VerifyBundle};
use cast::{executor::inspector::DEFAULT_CREATE2_DEPLOYER, CallKind};
use ethers::{
    abi::{Abi, Address},
    core::utils::to_checksum,
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

const DRY_RUN_DIR: &str = "dry-run";

/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
#[derive(Deserialize, Serialize, Clone)]
pub struct ScriptSequence {
    pub transactions: VecDeque<TransactionWithMetadata>,
    #[serde(serialize_with = "wrapper::serialize_receipts")]
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
        broadcasted: bool,
    ) -> eyre::Result<Self> {
        let path = ScriptSequence::get_path(&config.broadcast, sig, target, chain_id, broadcasted)?;
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
        broadcasted: bool,
    ) -> eyre::Result<Self> {
        let path = ScriptSequence::get_path(&config.broadcast, sig, target, chain_id, broadcasted)?;
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
        broadcasted: bool,
    ) -> eyre::Result<PathBuf> {
        let mut out = out.to_path_buf();
        let target_fname = target.source.file_name().wrap_err("No filename.")?;
        out.push(target_fname);
        out.push(chain_id.to_string());
        if !broadcasted {
            out.push(DRY_RUN_DIR);
        }

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
    #[serde(serialize_with = "wrapper::serialize_addr")]
    pub address: Address,
    #[serde(with = "hex")]
    pub init_code: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithMetadata {
    pub hash: Option<TxHash>,
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    #[serde(default = "default_string")]
    pub contract_name: Option<String>,
    #[serde(default = "default_address", serialize_with = "wrapper::serialize_opt_addr")]
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
        self.opcode = CallKind::Call;

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

// wrapper for modifying ethers-rs type serialization
mod wrapper {
    use super::*;
    use ethers::types::{Bloom, Bytes, Log, H256, U256, U64};

    pub fn serialize_addr<S>(addr: &Address, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&to_checksum(addr, None))
    }

    pub fn serialize_opt_addr<S>(opt: &Option<Address>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match opt {
            Some(addr) => serialize_addr(addr, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn serialize_vec_with_wrapped<S, T, WrappedType>(
        vec: &[T],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
        T: Clone,
        WrappedType: serde::Serialize + From<T>,
    {
        serializer.collect_seq(vec.iter().cloned().map(WrappedType::from))
    }

    // copied from https://github.com/gakonst/ethers-rs
    #[derive(Serialize, Deserialize)]
    struct WrappedLog {
        /// H160. the contract that emitted the log
        #[serde(serialize_with = "serialize_addr")]
        pub address: Address,

        /// topics: Array of 0 to 4 32 Bytes of indexed log arguments.
        /// (In solidity: The first topic is the hash of the signature of the event
        /// (e.g. `Deposit(address,bytes32,uint256)`), except you declared the event
        /// with the anonymous specifier.)
        pub topics: Vec<H256>,

        /// Data
        pub data: Bytes,

        /// Block Hash
        #[serde(rename = "blockHash")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub block_hash: Option<H256>,

        /// Block Number
        #[serde(rename = "blockNumber")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub block_number: Option<U64>,

        /// Transaction Hash
        #[serde(rename = "transactionHash")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub transaction_hash: Option<H256>,

        /// Transaction Index
        #[serde(rename = "transactionIndex")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub transaction_index: Option<U64>,

        /// Integer of the log index position in the block. None if it's a pending log.
        #[serde(rename = "logIndex")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub log_index: Option<U256>,

        /// Integer of the transactions index position log was created from.
        /// None when it's a pending log.
        #[serde(rename = "transactionLogIndex")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub transaction_log_index: Option<U256>,

        /// Log Type
        #[serde(rename = "logType")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub log_type: Option<String>,

        /// True when the log was removed, due to a chain reorganization.
        /// false if it's a valid log.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub removed: Option<bool>,
    }
    impl From<Log> for WrappedLog {
        fn from(log: Log) -> Self {
            Self {
                address: log.address,
                topics: log.topics,
                data: log.data,
                block_hash: log.block_hash,
                block_number: log.block_number,
                transaction_hash: log.transaction_hash,
                transaction_index: log.transaction_index,
                log_index: log.log_index,
                transaction_log_index: log.transaction_log_index,
                log_type: log.log_type,
                removed: log.removed,
            }
        }
    }

    fn serialize_logs<S: serde::Serializer>(
        logs: &[Log],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serialize_vec_with_wrapped::<S, Log, WrappedLog>(logs, serializer)
    }

    // "Receipt" of an executed transaction: details of its execution.
    // copied from https://github.com/gakonst/ethers-rs
    #[derive(Default, Clone, Serialize, Deserialize)]
    pub struct WrappedTransactionReceipt {
        /// Transaction hash.
        #[serde(rename = "transactionHash")]
        pub transaction_hash: H256,
        /// Index within the block.
        #[serde(rename = "transactionIndex")]
        pub transaction_index: U64,
        /// Hash of the block this transaction was included within.
        #[serde(rename = "blockHash")]
        pub block_hash: Option<H256>,
        /// Number of the block this transaction was included within.
        #[serde(rename = "blockNumber")]
        pub block_number: Option<U64>,
        /// address of the sender.
        #[serde(serialize_with = "serialize_addr")]
        pub from: Address,
        // address of the receiver. null when its a contract creation transaction.
        #[serde(serialize_with = "serialize_opt_addr")]
        pub to: Option<Address>,
        /// Cumulative gas used within the block after this was executed.
        #[serde(rename = "cumulativeGasUsed")]
        pub cumulative_gas_used: U256,
        /// Gas used by this transaction alone.
        ///
        /// Gas used is `None` if the the client is running in light client mode.
        #[serde(rename = "gasUsed")]
        pub gas_used: Option<U256>,
        /// Contract address created, or `None` if not a deployment.
        #[serde(rename = "contractAddress", serialize_with = "serialize_opt_addr")]
        pub contract_address: Option<Address>,
        /// Logs generated within this transaction.
        #[serde(serialize_with = "serialize_logs")]
        pub logs: Vec<Log>,
        /// Status: either 1 (success) or 0 (failure). Only present after activation of [EIP-658](https://eips.ethereum.org/EIPS/eip-658)
        pub status: Option<U64>,
        /// State root. Only present before activation of [EIP-658](https://eips.ethereum.org/EIPS/eip-658)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub root: Option<H256>,
        /// Logs bloom
        #[serde(rename = "logsBloom")]
        pub logs_bloom: Bloom,
        /// Transaction type, Some(1) for AccessList transaction, None for Legacy
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        pub transaction_type: Option<U64>,
        /// The price paid post-execution by the transaction (i.e. base fee + priority fee).
        /// Both fields in 1559-style transactions are *maximums* (max fee + max priority fee), the
        /// amount that's actually paid by users can only be determined post-execution
        #[serde(rename = "effectiveGasPrice", default, skip_serializing_if = "Option::is_none")]
        pub effective_gas_price: Option<U256>,
    }
    impl From<TransactionReceipt> for WrappedTransactionReceipt {
        fn from(receipt: TransactionReceipt) -> Self {
            Self {
                transaction_hash: receipt.transaction_hash,
                transaction_index: receipt.transaction_index,
                block_hash: receipt.block_hash,
                block_number: receipt.block_number,
                from: receipt.from,
                to: receipt.to,
                cumulative_gas_used: receipt.cumulative_gas_used,
                gas_used: receipt.gas_used,
                contract_address: receipt.contract_address,
                logs: receipt.logs,
                status: receipt.status,
                root: receipt.root,
                logs_bloom: receipt.logs_bloom,
                transaction_type: receipt.transaction_type,
                effective_gas_price: receipt.effective_gas_price,
            }
        }
    }

    pub fn serialize_receipts<S: serde::Serializer>(
        receipts: &[TransactionReceipt],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serialize_vec_with_wrapped::<S, TransactionReceipt, wrapper::WrappedTransactionReceipt>(
            receipts, serializer,
        )
    }
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
