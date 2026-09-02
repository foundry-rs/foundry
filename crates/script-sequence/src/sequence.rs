use crate::transaction::TransactionWithMetadata;
use alloy_network::{Network, ReceiptResponse};
use alloy_primitives::{TxHash, hex, map::HashMap};
use eyre::{ContextCompat, Result, WrapErr};
use foundry_common::{SELECTOR_LEN, TransactionMaybeSigned, fs, shell};
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DRY_RUN_DIR: &str = "dry-run";

#[derive(Clone, Serialize, Deserialize)]
pub struct NestedValue {
    pub internal_type: String,
    pub value: String,
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

/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
#[derive(Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "N::TransactionRequest: Serialize, N::TxEnvelope: Serialize",
    deserialize = "N::TransactionRequest: for<'de2> Deserialize<'de2>, N::TxEnvelope: for<'de2> Deserialize<'de2>"
))]
pub struct ScriptSequence<N: Network> {
    pub transactions: VecDeque<TransactionWithMetadata<N>>,
    pub receipts: Vec<N::ReceiptResponse>,
    pub libraries: Vec<String>,
    pub pending: Vec<TxHash>,
    #[serde(skip)]
    /// Contains paths to the sequence files
    /// None if sequence should not be saved to disk (e.g. part of a multi-chain sequence)
    pub paths: Option<(PathBuf, PathBuf)>,
    pub returns: HashMap<String, NestedValue>,
    pub timestamp: u128,
    pub chain: u64,
    pub commit: Option<String>,
}

impl<N: Network> Default for ScriptSequence<N> {
    fn default() -> Self {
        Self {
            transactions: Default::default(),
            receipts: Default::default(),
            libraries: Default::default(),
            pending: Default::default(),
            paths: Default::default(),
            returns: Default::default(),
            timestamp: Default::default(),
            chain: Default::default(),
            commit: Default::default(),
        }
    }
}

impl<N: Network> From<&ScriptSequence<N>> for SensitiveScriptSequence {
    fn from(sequence: &ScriptSequence<N>) -> Self {
        Self {
            transactions: sequence
                .transactions
                .iter()
                .map(|tx| SensitiveTransactionMetadata { rpc: tx.rpc.clone() })
                .collect(),
        }
    }
}

impl<N: Network> ScriptSequence<N> {
    /// Loads The sequence for the corresponding json file
    pub fn load(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
        dry_run: bool,
    ) -> Result<Self>
    where
        N::TxEnvelope: for<'d> Deserialize<'d>,
    {
        let (path, sensitive_path) = Self::get_paths(config, sig, target, chain_id, dry_run)?;

        let mut script_sequence: Self = fs::read_json_file(&path)
            .wrap_err(format!("Deployment not found for chain `{chain_id}`."))?;

        let sensitive_script_sequence: SensitiveScriptSequence = fs::read_json_file(
            &sensitive_path,
        )
        .wrap_err(format!("Deployment's sensitive details not found for chain `{chain_id}`."))?;

        script_sequence.fill_sensitive(&sensitive_script_sequence).wrap_err(format!(
            "Deployment's sensitive details are out of sync with the broadcast file for chain `{chain_id}`; the two were likely written partially (e.g. interrupted mid-save). Try re-running the deployment from scratch."
        ))?;

        script_sequence.paths = Some((path, sensitive_path));

        Ok(script_sequence)
    }

    /// Saves the transactions as file if it's a standalone deployment.
    /// `save_ts` should be set to true for checkpoint updates, which might happen many times and
    /// could result in us saving many identical files.
    pub fn save(&mut self, silent: bool, save_ts: bool) -> Result<()>
    where
        N::TxEnvelope: Serialize,
    {
        self.sort_receipts();

        if self.transactions.is_empty() {
            return Ok(());
        }

        self.timestamp = now().as_millis();
        let ts_name = format!("run-{}.json", self.timestamp);

        let sensitive_script_sequence = SensitiveScriptSequence::from(&*self);

        let Some((path, sensitive_path)) = self.paths.as_ref() else { return Ok(()) };

        // broadcast folder writes
        //../run-latest.json
        fs::write_pretty_json_file(path, &self)?;
        if save_ts {
            //../run-[timestamp].json
            fs::copy(path, path.with_file_name(&ts_name))?;
        }

        // cache folder writes
        //../run-latest.json
        fs::write_sensitive_json_file(sensitive_path, &sensitive_script_sequence)?;
        if save_ts {
            //../run-[timestamp].json
            fs::copy(sensitive_path, sensitive_path.with_file_name(&ts_name))?;
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

    pub fn add_receipt(&mut self, receipt: N::ReceiptResponse) {
        self.receipts.push(receipt);
    }

    /// Sorts all receipts with ascending transaction index
    pub fn sort_receipts(&mut self) {
        self.receipts.sort_by_key(|r| (r.block_number(), r.transaction_index()));
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
    /// `./broadcast/[contract_filename]/[chain_id]/[sig]-latest.json` and
    /// `./cache/[contract_filename]/[chain_id]/[sig]-latest.json`.
    pub fn get_paths(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
        dry_run: bool,
    ) -> Result<(PathBuf, PathBuf)> {
        let mut broadcast = config.broadcast.clone();
        let mut cache = config.cache_path.clone();
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
        let filename_with_ext = format!("{filename}-latest.json");

        broadcast.push(&filename_with_ext);
        cache.push(&filename_with_ext);

        Ok((broadcast, cache))
    }

    /// Returns the first RPC URL of this sequence.
    pub fn rpc_url(&self) -> &str {
        self.transactions.front().expect("empty sequence").rpc.as_str()
    }

    /// Returns the list of the transactions without the metadata.
    pub fn transactions(&self) -> impl Iterator<Item = &TransactionMaybeSigned<N>> {
        self.transactions.iter().map(|tx| tx.tx())
    }

    /// Fills each transaction's sensitive metadata (currently just the RPC url) from the
    /// corresponding entry in `sensitive`.
    ///
    /// Errors instead of panicking if `sensitive` has a different number of entries than
    /// `self.transactions` (either fewer OR more) - this can happen if the broadcast file and
    /// its sensitive-cache counterpart are written out of sync (e.g. the process is interrupted
    /// between the two separate writes in [`Self::save`], or a stale cache from a shorter prior
    /// run is left behind). The length check runs up front, before any mutation, so a mismatch
    /// never leaves `self.transactions` partially filled.
    pub fn fill_sensitive(&mut self, sensitive: &SensitiveScriptSequence) -> Result<()> {
        let transactions_len = self.transactions.len();
        let sensitive_len = sensitive.transactions.len();
        if transactions_len != sensitive_len {
            eyre::bail!(
                "sensitive-cache entry count ({sensitive_len}) does not match transaction count \
                 ({transactions_len}); the broadcast file and its sensitive-cache counterpart are \
                 out of sync"
            );
        }
        for (i, tx) in self.transactions.iter_mut().enumerate() {
            // Length equality was already checked above, so this index is always in bounds.
            tx.rpc.clone_from(&sensitive.transactions[i].rpc);
        }
        Ok(())
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
    if let Ok(calldata) = hex::decode(sig.strip_prefix("0x").unwrap_or(sig)) {
        // in which case we return the function selector if available
        if let Some(selector) = calldata.get(..SELECTOR_LEN) {
            return hex::encode(selector);
        }
        // fallback to original string if calldata is too short to contain selector
        return sig.to_string();
    }

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
        // valid calldata with 0x prefix
        assert_eq!(
            sig_to_file_name(
                "0x522bb704000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfFFb92266"
            )
            .as_str(),
            "522bb704"
        );
        // short calldata: should not panic and should return input as-is
        assert_eq!(sig_to_file_name("0x1234").as_str(), "0x1234");
        assert_eq!(sig_to_file_name("123").as_str(), "123");
        // invalid hex: should return input as-is
        assert_eq!(sig_to_file_name("0xnotahex").as_str(), "0xnotahex");
        // non-hex non-signature: should return input as-is
        assert_eq!(sig_to_file_name("not_a_sig_or_hex").as_str(), "not_a_sig_or_hex");
    }

    fn dummy_tx() -> TransactionWithMetadata<alloy_network::Ethereum> {
        TransactionWithMetadata {
            hash: None,
            call_kind: Default::default(),
            contract_name: None,
            contract_address: None,
            function: None,
            function_abi: None,
            display_function: None,
            arguments: None,
            rpc: String::new(),
            transaction: TransactionMaybeSigned::new(Default::default()),
            additional_contracts: vec![],
            is_fixed_gas_limit: false,
        }
    }

    /// A desynced sensitive-cache file (fewer entries than the broadcast file) must error
    /// instead of panicking on an out-of-bounds index - this is the actual state a partial
    /// write (e.g. an interrupted `save()`) can leave on disk.
    #[test]
    fn fill_sensitive_errors_on_desync_instead_of_panicking() {
        let mut sequence: ScriptSequence<alloy_network::Ethereum> = ScriptSequence::default();
        sequence.transactions.push_back(dummy_tx());
        sequence.transactions.push_back(dummy_tx());

        // Only one sensitive entry for two transactions - the desync case.
        let sensitive = SensitiveScriptSequence {
            transactions: [SensitiveTransactionMetadata { rpc: "http://a".to_string() }].into(),
        };

        let result = sequence.fill_sensitive(&sensitive);
        assert!(result.is_err(), "expected an error on a desynced sensitive cache, got Ok");
        // No mutation should have happened before the length check rejected the mismatch.
        assert_eq!(sequence.transactions[0].rpc, "");
    }

    /// A sensitive-cache file with MORE entries than the broadcast file must also error, not
    /// just the shorter case - a stale/leftover cache from a longer prior run is the same class
    /// of desync.
    #[test]
    fn fill_sensitive_errors_on_longer_cache_too() {
        let mut sequence: ScriptSequence<alloy_network::Ethereum> = ScriptSequence::default();
        sequence.transactions.push_back(dummy_tx());

        // Two sensitive entries for one transaction - the reverse desync case.
        let sensitive = SensitiveScriptSequence {
            transactions: [
                SensitiveTransactionMetadata { rpc: "http://a".to_string() },
                SensitiveTransactionMetadata { rpc: "http://b".to_string() },
            ]
            .into(),
        };

        let result = sequence.fill_sensitive(&sensitive);
        assert!(result.is_err(), "expected an error on a longer sensitive cache, got Ok");
    }

    /// Sanity check: a properly synced sensitive-cache file still fills correctly.
    #[test]
    fn fill_sensitive_fills_rpc_when_synced() {
        let mut sequence: ScriptSequence<alloy_network::Ethereum> = ScriptSequence::default();
        sequence.transactions.push_back(dummy_tx());
        sequence.transactions.push_back(dummy_tx());

        let sensitive = SensitiveScriptSequence {
            transactions: [
                SensitiveTransactionMetadata { rpc: "http://a".to_string() },
                SensitiveTransactionMetadata { rpc: "http://b".to_string() },
            ]
            .into(),
        };

        sequence.fill_sensitive(&sensitive).expect("synced cache must not error");
        assert_eq!(sequence.transactions[0].rpc, "http://a");
        assert_eq!(sequence.transactions[1].rpc, "http://b");
    }
}
