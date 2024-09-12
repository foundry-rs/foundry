use super::{multi_sequence::MultiChainSequence, NestedValue};
use crate::{
    transaction::{AdditionalContract, TransactionWithMetadata},
    verify::VerifyBundle,
};
use alloy_primitives::{hex, Address, TxHash};
use alloy_rpc_types::AnyTransactionReceipt;
use eyre::{eyre, ContextCompat, Result, WrapErr};
use forge_verify::provider::VerificationProviderType;
use foundry_cli::utils::{now, Git};
use foundry_common::{fs, shell, TransactionMaybeSigned, SELECTOR_LEN};
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
use yansi::Paint;

/// Returns the commit hash of the project if it exists
pub fn get_commit_hash(root: &Path) -> Option<String> {
    Git::new(root).commit_hash(true, "HEAD").ok()
}

pub enum ScriptSequenceKind {
    Single(ScriptSequence),
    Multi(MultiChainSequence),
}

impl ScriptSequenceKind {
    pub fn save(&mut self, silent: bool, save_ts: bool) -> Result<()> {
        match self {
            Self::Single(sequence) => sequence.save(silent, save_ts),
            Self::Multi(sequence) => sequence.save(silent, save_ts),
        }
    }

    pub fn sequences(&self) -> &[ScriptSequence] {
        match self {
            Self::Single(sequence) => std::slice::from_ref(sequence),
            Self::Multi(sequence) => &sequence.deployments,
        }
    }

    pub fn sequences_mut(&mut self) -> &mut [ScriptSequence] {
        match self {
            Self::Single(sequence) => std::slice::from_mut(sequence),
            Self::Multi(sequence) => &mut sequence.deployments,
        }
    }
    /// Updates underlying sequence paths to not be under /dry-run directory.
    pub fn update_paths_to_broadcasted(
        &mut self,
        config: &Config,
        sig: &str,
        target: &ArtifactId,
    ) -> Result<()> {
        match self {
            Self::Single(sequence) => {
                sequence.paths =
                    Some(ScriptSequence::get_paths(config, sig, target, sequence.chain, false)?);
            }
            Self::Multi(sequence) => {
                (sequence.path, sequence.sensitive_path) =
                    MultiChainSequence::get_paths(config, sig, target, false)?;
            }
        };

        Ok(())
    }
}

impl Drop for ScriptSequenceKind {
    fn drop(&mut self) {
        if let Err(err) = self.save(false, true) {
            error!(?err, "could not save deployment sequence");
        }
    }
}

pub const DRY_RUN_DIR: &str = "dry-run";

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

        let mut script_sequence: Self = foundry_compilers::utils::read_json_file(&path)
            .wrap_err(format!("Deployment not found for chain `{chain_id}`."))?;

        let sensitive_script_sequence: SensitiveScriptSequence =
            foundry_compilers::utils::read_json_file(&sensitive_path).wrap_err(format!(
                "Deployment's sensitive details not found for chain `{chain_id}`."
            ))?;

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
            return Ok(())
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
            shell::println(format!("\nTransactions saved to: {}\n", path.display()))?;
            shell::println(format!("Sensitive values saved to: {}\n", sensitive_path.display()))?;
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

    /// Given the broadcast log, it matches transactions with receipts, and tries to verify any
    /// created contract on etherscan.
    pub async fn verify_contracts(
        &mut self,
        config: &Config,
        mut verify: VerifyBundle,
    ) -> Result<()> {
        trace!(target: "script", "verifying {} contracts [{}]", verify.known_contracts.len(), self.chain);

        verify.set_chain(config, self.chain.into());

        if verify.etherscan.has_key() ||
            verify.verifier.verifier != VerificationProviderType::Etherscan
        {
            trace!(target: "script", "prepare future verifications");

            let mut future_verifications = Vec::with_capacity(self.receipts.len());
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
                if let (Some(address), Some(data)) = (receipt.contract_address, tx.tx().input()) {
                    match verify.get_verify_args(address, offset, data, &self.libraries) {
                        Some(verify) => future_verifications.push(verify.run()),
                        None => unverifiable_contracts.push(address),
                    };
                }

                // Verify potential contracts created during the transaction execution
                for AdditionalContract { address, init_code, .. } in &tx.additional_contracts {
                    match verify.get_verify_args(*address, 0, init_code.as_ref(), &self.libraries) {
                        Some(verify) => future_verifications.push(verify.run()),
                        None => unverifiable_contracts.push(*address),
                    };
                }
            }

            trace!(target: "script", "collected {} verification jobs and {} unverifiable contracts", future_verifications.len(), unverifiable_contracts.len());

            self.check_unverified(unverifiable_contracts, verify);

            let num_verifications = future_verifications.len();
            let mut num_of_successful_verifications = 0;
            println!("##\nStart verification for ({num_verifications}) contracts");
            for verification in future_verifications {
                match verification.await {
                    Ok(_) => {
                        num_of_successful_verifications += 1;
                    }
                    Err(err) => eprintln!("Error during verification: {err:#}"),
                }
            }

            if num_of_successful_verifications < num_verifications {
                return Err(eyre!("Not all ({num_of_successful_verifications} / {num_verifications}) contracts were verified!"))
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
                format!(
                    "We haven't found any matching bytecode for the following contracts: {:?}.\n\n{}",
                    unverifiable_contracts,
                    "This may occur when resuming a verification, but the underlying source code or compiler version has changed."
                )
                .yellow()
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
/// This accepts either the signature of the function or the raw calldata

pub fn sig_to_file_name(sig: &str) -> String {
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
