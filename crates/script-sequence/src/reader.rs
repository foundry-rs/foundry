use crate::{ScriptSequence, TransactionWithMetadata};
use alloy_rpc_types::AnyTransactionReceipt;
use eyre::{bail, Context, Result};
use foundry_common::fs;
use revm_inspectors::tracing::types::CallKind;
use std::path::{Path, PathBuf};

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
    tx_type: Option<CallKind>,
    broadcast_path: PathBuf,
}

impl BroadcastReader {
    /// Create a new `BroadcastReader` instance.
    pub fn new(contract_name: String, chain_id: u64, root: &Path) -> Result<Self> {
        let broadcast_path = root
            .join("broadcast")
            .join(format!("{contract_name}.s.sol"))
            .join(chain_id.to_string());

        if !broadcast_path.exists() {
            bail!("broadcast does not exist, ensure the contract name and/or chain_id is correct");
        }

        Ok(Self { contract_name, chain_id, tx_type: None, broadcast_path })
    }

    /// Set the transaction type to filter by.
    pub fn with_tx_type(mut self, tx_type: CallKind) -> Self {
        self.tx_type = Some(tx_type);
        self
    }

    /// Read all broadcast files in the broadcast directory.
    pub fn read_all(&self) -> eyre::Result<Vec<ScriptSequence>> {
        let files = std::fs::read_dir(&self.broadcast_path)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.file_name()?.to_string_lossy() == "run-latest.json" {
                    return None;
                }
                Some(path)
            })
            .collect::<Vec<_>>();

        let broadcasts = files
            .iter()
            .map(|path| fs::read_json_file(path).wrap_err("failed reading broadcast"))
            .collect::<Result<Vec<ScriptSequence>>>()?;

        Ok(broadcasts)
    }

    /// Attempts read the latest broadcast file in the broadcast directory.
    ///
    /// This may be the `run-latest.json` file or the broadcast file with the latest timestamp.
    pub fn read_latest(&self) -> eyre::Result<ScriptSequence> {
        let latest_broadcast: Result<ScriptSequence> =
            fs::read_json_file(&self.broadcast_path.join("run-latest.json"))
                .wrap_err("failed reading latest broadcast");

        if let Ok(latest_broadcast) = latest_broadcast {
            return Ok(latest_broadcast);
        }

        // Iterate over the files in the broadcast path directory except for the run-latest.json
        let files = std::fs::read_dir(&self.broadcast_path)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.file_name()?.to_string_lossy() == "run-latest.json" {
                    return None;
                }
                Some(path)
            })
            .collect::<Vec<_>>();

        let broadcasts = files
            .iter()
            .map(|path| fs::read_json_file(path).wrap_err("failed reading broadcast"))
            .collect::<Result<Vec<ScriptSequence>>>()?;

        // Find the broadcast with the latest timestamp
        let target = broadcasts
            .into_iter()
            .max_by_key(|broadcast| broadcast.timestamp)
            .ok_or_else(|| eyre::eyre!("No broadcasts found"))?;

        Ok(target)
    }

    /// Search for transactions in the broadcast that match the specified `contractName` and
    /// `txType`.
    pub fn search_broadcast(
        &self,
        broadcast: ScriptSequence,
    ) -> Result<Vec<(TransactionWithMetadata, AnyTransactionReceipt)>> {
        let transactions = broadcast.transactions.clone();

        if broadcast.chain == self.chain_id {
            let txs = transactions
                .into_iter()
                .filter(|tx| {
                    let name_filter =
                        tx.contract_name.clone().is_some_and(|cn| cn == self.contract_name);

                    let type_filter = self.tx_type.map_or(true, |kind| tx.opcode == kind);

                    name_filter && type_filter
                })
                .collect::<Vec<_>>();

            let mut targets = Vec::new();
            for tx in txs.into_iter() {
                broadcast.receipts.iter().for_each(|receipt| {
                    if tx.hash.is_some_and(|hash| hash == receipt.transaction_hash) {
                        targets.push((tx.clone(), receipt.clone()));
                    }
                });
            }

            if !targets.is_empty() {
                return Ok(targets);
            }
        }

        bail!("target tx not found in broadcast on chain: {}", self.chain_id);
    }
}
