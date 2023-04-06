use super::NestedValue;
use crate::cmd::forge::{
    init::get_commit_hash,
    script::{
        transaction::{wrapper, AdditionalContract, TransactionWithMetadata},
        verify::VerifyBundle,
    },
    verify::provider::VerificationProviderType,
};
use ethers::{
    abi::Address,
    prelude::{artifacts::Libraries, ArtifactId, TransactionReceipt, TxHash},
    types::transaction::eip2718::TypedTransaction,
};
use eyre::{ContextCompat, WrapErr};
use foundry_common::{fs, shell, SELECTOR_LEN};
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io::BufWriter,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::trace;
use yansi::Paint;

pub const DRY_RUN_DIR: &str = "dry-run";

/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
#[derive(Deserialize, Serialize, Clone, Default)]
pub struct ScriptSequence {
    pub transactions: VecDeque<TransactionWithMetadata>,
    #[serde(serialize_with = "wrapper::serialize_receipts")]
    pub receipts: Vec<TransactionReceipt>,
    pub libraries: Vec<String>,
    pub pending: Vec<TxHash>,
    pub path: PathBuf,
    pub returns: HashMap<String, NestedValue>,
    pub timestamp: u64,
    pub chain: u64,
    /// If `True`, the sequence belongs to a `MultiChainSequence` and won't save to disk as usual.
    pub multi: bool,
    pub commit: Option<String>,
}

impl ScriptSequence {
    pub fn new(
        transactions: VecDeque<TransactionWithMetadata>,
        returns: HashMap<String, NestedValue>,
        sig: &str,
        target: &ArtifactId,
        config: &Config,
        broadcasted: bool,
        is_multi: bool,
    ) -> eyre::Result<Self> {
        let chain = config.chain_id.unwrap_or_default().id();

        let path = ScriptSequence::get_path(
            &config.broadcast,
            sig,
            target,
            chain,
            broadcasted && !is_multi,
        )?;
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
            chain,
            multi: is_multi,
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
        ethers::solc::utils::read_json_file(path)
            .wrap_err(format!("Deployment not found for chain `{chain_id}`."))
    }

    /// Saves the transactions as file if it's a standalone deployment.
    pub fn save(&mut self) -> eyre::Result<()> {
        if !self.multi && !self.transactions.is_empty() {
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
            shell::println(format!("\nTransactions saved to: {path}\n"))?;
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
                libs.iter()
                    .map(|(name, address)| format!("{}:{name}:{address}", file.to_string_lossy()))
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
    pub async fn verify_contracts(
        &mut self,
        config: &Config,
        mut verify: VerifyBundle,
    ) -> eyre::Result<()> {
        trace!(target: "script", "verifying {} contracts [{}]", verify.known_contracts.len(), self.chain);

        verify.set_chain(config, self.chain.into());

        if verify.etherscan.key.is_some() ||
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

            trace!(target: "script", "collected {} verification jobs and {} unverifiable contracts", future_verifications.len(), unverifiable_contracts.len());

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
    pub fn typed_transactions(&self) -> Vec<(String, &TypedTransaction)> {
        self.transactions
            .iter()
            .map(|tx| {
                (tx.rpc.clone().expect("to have been filled with a proper rpc"), tx.typed_tx())
            })
            .collect()
    }
}

impl Drop for ScriptSequence {
    fn drop(&mut self) {
        self.sort_receipts();
        self.save().expect("not able to save deployment sequence");
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
