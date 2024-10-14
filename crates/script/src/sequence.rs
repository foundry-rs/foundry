use crate::{multi_sequence::MultiChainSequence, verify::VerifyBundle};
use alloy_primitives::{Address, TxHash};
use alloy_rpc_types::AnyTransactionReceipt;
use eyre::{eyre, Result};
use forge_script_sequence::{
    transaction::AdditionalContract, ScriptSequence, SensitiveScriptSequence,
};
use forge_verify::provider::VerificationProviderType;
use foundry_cli::utils::Git;
use foundry_common::TransactionMaybeSigned;
use foundry_compilers::ArtifactId;
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::path::Path;
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

/// Helper that saves the transactions sequence and its state on which transactions have been
/// broadcasted
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ScriptSequenceManager {
    inner: ScriptSequence,
}

impl ScriptSequenceManager {
    /// Creates a new instance of the script sequence manager
    pub fn new(inner: ScriptSequence) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &ScriptSequence {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut ScriptSequence {
        &mut self.inner
    }

    /// Loads The sequence for the corresponding json file
    pub fn load(
        config: &Config,
        sig: &str,
        target: &ArtifactId,
        chain_id: u64,
        dry_run: bool,
    ) -> Result<Self> {
        let script_seq = ScriptSequence::load(config, sig, target, chain_id, dry_run)?;

        Ok(Self { inner: script_seq })
    }

    /// Saves the transactions as file if it's a standalone deployment.
    /// `save_ts` should be set to true for checkpoint updates, which might happen many times and
    /// could result in us saving many identical files.
    pub fn save(&mut self, silent: bool, save_ts: bool) -> Result<()> {
        self.inner.save(silent, save_ts)?;

        Ok(())
    }

    pub fn add_receipt(&mut self, receipt: AnyTransactionReceipt) {
        self.inner.receipts.push(receipt);
    }

    /// Sorts all receipts with ascending transaction index
    pub fn sort_receipts(&mut self) {
        self.inner.receipts.sort_by_key(|r| (r.block_number, r.transaction_index));
    }

    pub fn add_pending(&mut self, index: usize, tx_hash: TxHash) {
        if !self.inner.pending.contains(&tx_hash) {
            self.inner.transactions[index].hash = Some(tx_hash);
            self.inner.pending.push(tx_hash);
        }
    }

    pub fn remove_pending(&mut self, tx_hash: TxHash) {
        self.inner.pending.retain(|element| element != &tx_hash);
    }

    /// Given the broadcast log, it matches transactions with receipts, and tries to verify any
    /// created contract on etherscan.
    pub async fn verify_contracts(
        &mut self,
        config: &Config,
        mut verify: VerifyBundle,
    ) -> Result<()> {
        trace!(target: "script", "verifying {} contracts [{}]", verify.known_contracts.len(), self.inner.chain);

        verify.set_chain(config, self.inner.chain.into());

        if verify.etherscan.has_key() ||
            verify.verifier.verifier != VerificationProviderType::Etherscan
        {
            trace!(target: "script", "prepare future verifications");

            let mut future_verifications = Vec::with_capacity(self.inner.receipts.len());
            let mut unverifiable_contracts = vec![];

            // Make sure the receipts have the right order first.
            self.sort_receipts();

            for (receipt, tx) in self.inner.receipts.iter_mut().zip(self.inner.transactions.iter())
            {
                // create2 hash offset
                let mut offset = 0;

                if tx.is_create2() {
                    receipt.contract_address = tx.contract_address;
                    offset = 32;
                }

                // Verify contract created directly from the transaction
                if let (Some(address), Some(data)) = (receipt.contract_address, tx.tx().input()) {
                    match verify.get_verify_args(address, offset, data, &self.inner.libraries) {
                        Some(verify) => future_verifications.push(verify.run()),
                        None => unverifiable_contracts.push(address),
                    };
                }

                // Verify potential contracts created during the transaction execution
                for AdditionalContract { address, init_code, .. } in &tx.additional_contracts {
                    match verify.get_verify_args(
                        *address,
                        0,
                        init_code.as_ref(),
                        &self.inner.libraries,
                    ) {
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

            if let Some(commit) = &self.inner.commit {
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
        self.inner.transactions.front().expect("empty sequence").rpc.as_str()
    }

    /// Returns the list of the transactions without the metadata.
    pub fn transactions(&self) -> impl Iterator<Item = &TransactionMaybeSigned> {
        self.inner.transactions.iter().map(|tx| tx.tx())
    }

    pub fn fill_sensitive(&mut self, sensitive: &SensitiveScriptSequence) {
        self.inner
            .transactions
            .iter_mut()
            .enumerate()
            .for_each(|(i, tx)| tx.rpc.clone_from(&sensitive.transactions[i].rpc));
    }
}
