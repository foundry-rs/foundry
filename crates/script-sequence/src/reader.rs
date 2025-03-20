use crate::{ScriptSequence, TransactionWithMetadata};
use alloy_network::AnyTransactionReceipt;
use eyre::{bail, Result};
use foundry_common::fs;
use revm_inspectors::tracing::types::CallKind;
use std::path::{Component, Path, PathBuf};

/// This type reads broadcast files in the
/// `project_root/broadcast/{contract_name}.s.sol/{chain_id}/` directory.
///
/// It consists methods that filter and search for transactions in the broadcast files that match a
/// `transactionType` if provided.
///
/// Note:
///
/// It only returns transactions for which there exists a corresponding receipt in the broadcast.
#[derive(Debug, Clone)]
pub struct BroadcastReader {
    contract_name: String,
    chain_id: u64,
    tx_type: Vec<CallKind>,
    broadcast_path: PathBuf,
}

impl BroadcastReader {
    /// Create a new `BroadcastReader` instance.
    pub fn new(contract_name: String, chain_id: u64, broadcast_path: &Path) -> Result<Self> {
        if !broadcast_path.exists() && !broadcast_path.is_dir() {
            bail!("broadcast dir does not exist");
        }

        Ok(Self {
            contract_name,
            chain_id,
            tx_type: Default::default(),
            broadcast_path: broadcast_path.to_path_buf(),
        })
    }

    /// Set the transaction type to filter by.
    pub fn with_tx_type(mut self, tx_type: CallKind) -> Self {
        self.tx_type.push(tx_type);
        self
    }

    /// Read all broadcast files in the broadcast directory.
    ///
    /// Example structure:
    ///
    /// project-root/broadcast/{script_name}.s.sol/{chain_id}/*.json
    /// project-root/broadcast/multi/{multichain_script_name}.s.sol-{timestamp}/deploy.json
    pub fn read(&self) -> eyre::Result<Vec<ScriptSequence>> {
        // 1. Recursively read all .json files in the broadcast directory
        let mut broadcasts = vec![];
        for entry in walkdir::WalkDir::new(&self.broadcast_path).into_iter() {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
                // Ignore -latest to avoid duplicating broadcast entries
                if path.components().any(|c| c.as_os_str().to_string_lossy().contains("-latest")) {
                    continue;
                }

                // Detect Multichain broadcasts using "multi" in the path
                if path.components().any(|c| c == Component::Normal("multi".as_ref())) {
                    // Parse as MultiScriptSequence

                    let broadcast = fs::read_json_file::<serde_json::Value>(path)?;
                    let multichain_deployments = broadcast
                        .get("deployments")
                        .and_then(|deployments| {
                            serde_json::from_value::<Vec<ScriptSequence>>(deployments.clone()).ok()
                        })
                        .unwrap_or_default();

                    broadcasts.extend(multichain_deployments);
                    continue;
                }

                let broadcast = fs::read_json_file::<ScriptSequence>(path)?;
                broadcasts.push(broadcast);
            }
        }

        let broadcasts = self.filter_and_sort(broadcasts);

        Ok(broadcasts)
    }

    /// Attempts read the latest broadcast file in the broadcast directory.
    ///
    /// This may be the `run-latest.json` file or the broadcast file with the latest timestamp.
    pub fn read_latest(&self) -> eyre::Result<ScriptSequence> {
        let broadcasts = self.read()?;

        // Find the broadcast with the latest timestamp
        let target = broadcasts
            .into_iter()
            .max_by_key(|broadcast| broadcast.timestamp)
            .ok_or_else(|| eyre::eyre!("No broadcasts found"))?;

        Ok(target)
    }

    /// Applies the filters and sorts the broadcasts by descending timestamp.
    pub fn filter_and_sort(&self, broadcasts: Vec<ScriptSequence>) -> Vec<ScriptSequence> {
        // Apply the filters
        let mut seqs = broadcasts
            .into_iter()
            .filter(|broadcast| {
                if broadcast.chain != self.chain_id {
                    return false;
                }

                broadcast.transactions.iter().any(move |tx| {
                    let name_filter =
                        tx.contract_name.clone().is_some_and(|cn| cn == self.contract_name);

                    let type_filter = self.tx_type.is_empty() || self.tx_type.contains(&tx.opcode);

                    name_filter && type_filter
                })
            })
            .collect::<Vec<_>>();

        // Sort by descending timestamp
        seqs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        seqs
    }

    /// Search for transactions in the broadcast that match the specified `contractName` and
    /// `txType`.
    ///
    /// It cross-checks the transactions with their corresponding receipts in the broadcast and
    /// returns the result.
    ///
    /// Transactions that don't have a corresponding receipt are ignored.
    ///
    /// Sorts the transactions by descending block number.
    pub fn into_tx_receipts(
        &self,
        broadcast: ScriptSequence,
    ) -> Vec<(TransactionWithMetadata, AnyTransactionReceipt)> {
        let transactions = broadcast.transactions.clone();

        let txs = transactions
            .into_iter()
            .filter(|tx| {
                let name_filter =
                    tx.contract_name.clone().is_some_and(|cn| cn == self.contract_name);

                let type_filter = self.tx_type.is_empty() || self.tx_type.contains(&tx.opcode);

                name_filter && type_filter
            })
            .collect::<Vec<_>>();

        let mut targets = Vec::new();
        for tx in txs.into_iter() {
            let maybe_receipt = broadcast
                .receipts
                .iter()
                .find(|receipt| tx.hash.is_some_and(|hash| hash == receipt.transaction_hash));

            if let Some(receipt) = maybe_receipt {
                targets.push((tx, receipt.clone()));
            }
        }

        // Sort by descending block number
        targets.sort_by(|a, b| b.1.block_number.cmp(&a.1.block_number));

        targets
    }
}
