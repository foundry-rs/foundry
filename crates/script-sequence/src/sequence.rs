use crate::transaction::TransactionWithMetadata;
use alloy_network::AnyTransactionReceipt;
use alloy_primitives::{hex, map::HashMap, TxHash};
use eyre::{ContextCompat, Result, WrapErr};
use foundry_common::{fs, shell, TransactionMaybeSigned, SELECTOR_LEN};
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{BufWriter, Write},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DRY_RUN_DIR: &str = "dry-run";

#[derive(Clone, Serialize, Deserialize)]
pub struct NestedValue {
    pub internal_type: String,
    pub value: String,
}

/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ScriptSequence {
    pub transactions: VecDeque<TransactionWithMetadata>,
    pub receipts: Vec<AnyTransactionReceipt>,
    pub libraries: Vec<String>,
    pub pending: Vec<TxHash>,
    #[serde(skip)]
    /// Contains paths to the sequence files
    /// None if sequence should not be saved to disk (e.g. part of a multi-chain sequence)
    pub paths: Option<(PathBuf, PathBuf)>,
    pub returns: HashMap<String, NestedValue>,
    pub timestamp: u64,
    pub chain: u64,
    pub commit: Option<String>,
}

/// Sensitive values from the transactions in a script sequence
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SensitiveTransactionMetadata {
    pub rpc: String,
}

/// Sensitive info from the script sequence which is saved into the cache folder
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SensitiveScriptSequence {
    pub transactions: VecDeque<SensitiveTransactionMetadata>,
}

impl From<ScriptSequence> for SensitiveScriptSequence {
    fn from(sequence: ScriptSequence) -> Self {
        Self {
            transactions: sequence
                .transactions
                .iter()
                .map(|tx| SensitiveTransactionMetadata { rpc: tx.rpc.clone() })
                .collect(),
        }
    }
}

impl ScriptSequence {
    /// Loads The sequence for the corresponding json file
    pub fn load(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
        dry_run: bool,
    ) -> Result<Self> {
        let (path, sensitive_path) = Self::get_paths(config, sig, target, chain_id, dry_run)?;

        let mut script_sequence: Self = fs::read_json_file(&path)
            .wrap_err(format!("Deployment not found for chain `{chain_id}`."))?;

        let sensitive_script_sequence: SensitiveScriptSequence = fs::read_json_file(
            &sensitive_path,
        )
        .wrap_err(format!("Deployment's sensitive details not found for chain `{chain_id}`."))?;

        script_sequence.fill_sensitive(&sensitive_script_sequence);

        script_sequence.paths = Some((path, sensitive_path));

        Ok(script_sequence)
    }

    /// Saves the transactions as file if it's a standalone deployment.
    /// `save_ts` should be set to true for checkpoint updates, which might happen many times and
    /// could result in us saving many identical files.
    pub fn save(&mut self, silent: bool, save_ts: bool) -> Result<()> {
        self.sort_receipts();

        if self.transactions.is_empty() {
            return Ok(());
        }

        let Some((path, sensitive_path)) = self.paths.clone() else { return Ok(()) };

        self.timestamp = now().as_secs();
        let ts_name = format!("run-{}.json", self.timestamp);

        let sensitive_script_sequence: SensitiveScriptSequence = self.clone().into();

        // broadcast folder writes
        //../run-latest.json
        let mut writer = BufWriter::new(fs::create_file(&path)?);
        serde_json::to_writer_pretty(&mut writer, &self)?;
        writer.flush()?;
        if save_ts {
            //../run-[timestamp].json
            fs::copy(&path, path.with_file_name(&ts_name))?;
        }

        // cache folder writes
        //../run-latest.json
        let mut writer = BufWriter::new(fs::create_file(&sensitive_path)?);
        serde_json::to_writer_pretty(&mut writer, &sensitive_script_sequence)?;
        writer.flush()?;
        if save_ts {
            //../run-[timestamp].json
            fs::copy(&sensitive_path, sensitive_path.with_file_name(&ts_name))?;
        }

        if !silent {
            if shell::is_json() {
                sh_println!(
                    "{}",
                    serde_json::json!({
                        "status": "success",
                        "transactions": path.display().to_string(),
                        "sensitive": sensitive_path.display().to_string(),
                    })
                )?;
            } else {
                sh_println!("\nTransactions saved to: {}\n", path.display())?;
                sh_println!("Sensitive values saved to: {}\n", sensitive_path.display())?;
            }
        }

        Ok(())
    }

    pub fn add_receipt(&mut self, receipt: AnyTransactionReceipt) {
        self.receipts.push(receipt);
    }

    /// Sorts all receipts with ascending transaction index
    pub fn sort_receipts(&mut self) {
        self.receipts.sort_by_key(|r| (r.block_number, r.transaction_index));
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

    /// Gets paths in the formats
    /// `./broadcast/[contract_filename]/[chain_id]/[sig]-[timestamp].json` and
    /// `./cache/[contract_filename]/[chain_id]/[sig]-[timestamp].json`.
    pub fn get_paths(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
        dry_run: bool,
    ) -> Result<(PathBuf, PathBuf)> {
        let mut broadcast = config.broadcast.to_path_buf();
        let mut cache = config.cache_path.to_path_buf();
        let mut common = PathBuf::new();

        let target_fname = target.source.file_name().wrap_err("No filename.")?;
        common.push(target_fname);
        common.push(chain_id.to_string());
        if dry_run {
            common.push(DRY_RUN_DIR);
        }

        broadcast.push(common.clone());
        cache.push(common);

        fs::create_dir_all(&broadcast)?;
        fs::create_dir_all(&cache)?;

        // TODO: ideally we want the name of the function here if sig is calldata
        let filename = sig_to_file_name(sig);

        broadcast.push(format!("{filename}-latest.json"));
        cache.push(format!("{filename}-latest.json"));

        Ok((broadcast, cache))
    }

    /// Returns the first RPC URL of this sequence.
    pub fn rpc_url(&self) -> &str {
        self.transactions.front().expect("empty sequence").rpc.as_str()
    }

    /// Returns the list of the transactions without the metadata.
    pub fn transactions(&self) -> impl Iterator<Item = &TransactionMaybeSigned> {
        self.transactions.iter().map(|tx| tx.tx())
    }

    pub fn fill_sensitive(&mut self, sensitive: &SensitiveScriptSequence) {
        self.transactions
            .iter_mut()
            .enumerate()
            .for_each(|(i, tx)| tx.rpc.clone_from(&sensitive.transactions[i].rpc));
    }
}

/// Converts the `sig` argument into the corresponding file path.
///
/// This accepts either the signature of the function or the raw calldata.
pub fn sig_to_file_name(sig: &str) -> String {
    if let Some((name, _)) = sig.split_once('(') {
        // strip until call argument parenthesis
        return name.to_string();
    }
    // assume calldata if `sig` is hex
    if let Ok(calldata) = hex::decode(sig) {
        // in which case we return the function signature
        return hex::encode(&calldata[..SELECTOR_LEN]);
    }

    // return sig as is
    sig.to_string()
}

pub fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards")
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
