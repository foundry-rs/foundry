use crate::{ScriptSequence, TransactionWithMetadata};
use alloy_rpc_types::AnyTransactionReceipt;
use eyre::{bail, Result};
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
    pub fn new(contract_name: String, chain_id: u64, broadcast_path: &Path) -> Result<Self> {
        if !broadcast_path.exists() && !broadcast_path.is_dir() {
            bail!("broadcast does not exist, ensure the contract name and/or chain_id is correct");
        }

        Ok(Self {
            contract_name,
            chain_id,
            tx_type: None,
            broadcast_path: broadcast_path.to_path_buf(),
        })
    }

    /// Set the transaction type to filter by.
    pub fn with_tx_type(mut self, tx_type: CallKind) -> Self {
        self.tx_type = Some(tx_type);
        self
    }

    /// Read all broadcast files in the broadcast directory.
    ///
    /// Example structure:
    ///
    /// project-root/broadcast/{script_name}.s.sol/{chain_id}/*.json
    /// project-root/broadcast/multi/{multichain_script_name}.s.sol-{timestamp}/deploy.json
    pub fn read_all(&self) -> eyre::Result<Vec<ScriptSequence>> {
        // 1. Read the broadcast directory and get all the script directories and multi dirs i.e
        //    {script_contract_name}.s.sol/ OR broadcast/multi/
        let script_dirs = std::fs::read_dir(&self.broadcast_path)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                // Get all the script directories that end with `.s.sol`
                if path.is_dir() &&
                    path.file_name()
                        .is_some_and(|dir_name| dir_name.to_string_lossy().ends_with(".s.sol"))
                {
                    return Some(path);
                }
                None
            })
            .collect::<Vec<_>>();

        // 2. Iterate over the script directories and get the {chain_id} directories
        let chain_dirs = script_dirs
            .into_iter()
            .filter_map(|script_dir| {
                std::fs::read_dir(&script_dir).ok().map(|read_dir| {
                    read_dir.filter_map(|chain_dir| {
                        let chain_dir = chain_dir.ok()?;
                        let path = chain_dir.path();

                        // Get all the chain directories that match the chain_id
                        if path.is_dir() &&
                            path.file_name().is_some_and(|dir_name| {
                                dir_name.to_string_lossy() == self.chain_id.to_string()
                            })
                        {
                            Some(path)
                        } else {
                            None
                        }
                    })
                })
            })
            .flatten()
            .collect::<Vec<_>>();

        // 3. Iterate over the chain directories and get all the broadcast files
        let broadcasts = chain_dirs
            .into_iter()
            .flat_map(|chain_dir| {
                fs::json_files(&chain_dir).filter_map(|path| {
                    // Ignore if file == run-latest.json to avoid duplicates
                    if path.file_name().is_some_and(|file| file == "run-latest.json") {
                        return None;
                    }
                    fs::read_json_file::<ScriptSequence>(&path).ok().filter(|broadcast| {
                        if broadcast.chain != self.chain_id {
                            return false;
                        }
                        broadcast.transactions.iter().any(move |tx| {
                            tx.contract_name.as_ref().is_some_and(|cn| {
                                cn == &self.contract_name &&
                                    self.tx_type.map_or(true, |kind| tx.opcode == kind)
                            })
                        })
                    })
                })
            })
            .collect::<Vec<_>>();

        let multichain_broadcasts = self.read_multi()?;

        let broadcasts =
            broadcasts.into_iter().chain(multichain_broadcasts.into_iter()).collect::<Vec<_>>();

        Ok(broadcasts)
    }

    pub fn read_multi(&self) -> eyre::Result<Vec<ScriptSequence>> {
        // Read multichain broadcasts
        let multi_dir = self.broadcast_path.join("multi");

        let multi_chain_dirs = if multi_dir.exists() && multi_dir.is_dir() {
            std::fs::read_dir(&multi_dir)?
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let path = entry.path();

                    if !path.is_dir() {
                        return None
                    }

                    let file = path.file_name();

                    // Ignore -latest to avoid duplicating entries
                    if file.is_some_and(|dir_name| {
                        dir_name.to_string_lossy().ends_with(".s.sol-latest")
                    }) {
                        return None;
                    }

                    // Get all the multi chain directories that end with `.s.sol`
                    if file.is_some_and(|dir_name| dir_name.to_string_lossy().contains(".s.sol")) {
                        return Some(path);
                    }
                    None
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        let multichain_seqs = multi_chain_dirs
            .into_iter()
            .flat_map(|multi_dir| fs::json_files(&multi_dir))
            .collect::<Vec<_>>();

        let seqs = multichain_seqs
            .into_iter()
            .flat_map(|path| {
                fs::read_json_file::<serde_json::Value>(&path).ok().and_then(|ser_seqs| {
                    Some(
                        ser_seqs
                            .get("deployments")
                            .and_then(|deployments| {
                                serde_json::from_value::<Vec<ScriptSequence>>(deployments.clone())
                                    .ok()
                            })
                            .unwrap_or_default(),
                    )
                })
            })
            .flatten()
            .collect::<Vec<_>>();

        // Apply the filters
        let seqs = seqs
            .into_iter()
            .filter(|broadcast| {
                if broadcast.chain != self.chain_id {
                    return false;
                }

                broadcast.transactions.iter().any(move |tx| {
                    tx.contract_name.as_ref().is_some_and(|cn| {
                        cn == &self.contract_name &&
                            self.tx_type.map_or(true, |kind| tx.opcode == kind)
                    })
                })
            })
            .collect::<Vec<_>>();

        Ok(seqs)
    }

    /// Attempts read the latest broadcast file in the broadcast directory.
    ///
    /// This may be the `run-latest.json` file or the broadcast file with the latest timestamp.
    pub fn read_latest(&self) -> eyre::Result<ScriptSequence> {
        let broadcasts = self.read_all()?;

        // Find the broadcast with the latest timestamp
        let target = broadcasts
            .into_iter()
            .max_by_key(|broadcast| broadcast.timestamp)
            .ok_or_else(|| eyre::eyre!("No broadcasts found"))?;

        Ok(target)
    }

    /// Search for transactions in the broadcast that match the specified `contractName` and
    /// `txType`.
    ///
    /// It cross-checks the transactions with their corresponding receipts in the broadcast and
    /// returns the result.
    ///
    /// Transactions that don't have a corresponding receipt are ignored.
    pub fn search_broadcast(
        &self,
        broadcast: ScriptSequence,
    ) -> Vec<(TransactionWithMetadata, AnyTransactionReceipt)> {
        let transactions = broadcast.transactions.clone();

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

        // Sort by descending block number
        targets.sort_by(|a, b| b.1.block_number.cmp(&a.1.block_number));

        targets
    }
}
