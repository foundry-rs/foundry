use crate::{ScriptSequence, TransactionWithMetadata};
use alloy_network::{Network, ReceiptResponse};
use alloy_primitives::Address;
use eyre::{Result, bail};
use foundry_common::{errors::FsPathError, fs};
use revm_inspectors::tracing::types::CallKind;
use serde::Deserialize;
use serde_json::Value;
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
    match_labels: bool,
}

impl BroadcastReader {
    /// Create a new `BroadcastReader` instance.
    pub fn new(contract_name: String, chain_id: u64, broadcast_path: &Path) -> Result<Self> {
        if !broadcast_path.is_dir() {
            bail!("broadcast dir does not exist or is not a directory");
        }

        Ok(Self {
            contract_name,
            chain_id,
            tx_type: Default::default(),
            broadcast_path: broadcast_path.to_path_buf(),
            match_labels: false,
        })
    }

    /// Set the transaction type to filter by.
    pub fn with_tx_type(mut self, tx_type: CallKind) -> Self {
        self.tx_type.push(tx_type);
        self
    }

    fn matches_filters<N: Network>(&self, tx: &TransactionWithMetadata<N>) -> bool {
        let name_filter = tx.contract_name.as_ref().is_some_and(|cn| *cn == self.contract_name)
            || (self.match_labels
                && tx.label.as_ref().is_some_and(|label| *label == self.contract_name));
        let type_filter = self.tx_type.is_empty() || self.tx_type.contains(&tx.call_kind);
        name_filter && type_filter
    }

    /// Read all broadcast files in the broadcast directory.
    ///
    /// Example structure:
    ///
    /// project-root/broadcast/{script_name}.s.sol/{chain_id}/*.json
    /// project-root/broadcast/multi/{multichain_script_name}.s.sol-{timestamp}/deploy.json
    pub fn read<N: Network>(&self) -> eyre::Result<Vec<ScriptSequence<N>>>
    where
        N::TxEnvelope: for<'d> Deserialize<'d>,
    {
        // 1. Recursively read all .json files in the broadcast directory
        let mut broadcasts = vec![];
        for entry in walkdir::WalkDir::new(&self.broadcast_path) {
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

                    let broadcast = fs::read_json_file::<Value>(path)?;
                    let multichain_deployments = broadcast
                        .get("deployments")
                        .and_then(Value::as_array)
                        .and_then(|deployments| {
                            deployments
                                .iter()
                                .cloned()
                                .map(parse_sequence)
                                .collect::<serde_json::Result<Vec<_>>>()
                                .ok()
                        })
                        .unwrap_or_default();

                    broadcasts.extend(multichain_deployments);
                    continue;
                }

                let broadcast = parse_sequence(fs::read_json_file(path)?)
                    .map_err(|source| FsPathError::ReadJson { source, path: path.into() })?;
                broadcasts.push(broadcast);
            }
        }

        let broadcasts = self.filter_and_sort(broadcasts);

        Ok(broadcasts)
    }

    /// Attempts read the latest broadcast file in the broadcast directory.
    ///
    /// This may be the `run-latest.json` file or the broadcast file with the latest timestamp.
    pub fn read_latest<N: Network>(&self) -> eyre::Result<ScriptSequence<N>>
    where
        N::TxEnvelope: for<'d> Deserialize<'d>,
    {
        let broadcasts = self.read()?;

        // Find the broadcast with the latest timestamp
        let target = broadcasts
            .into_iter()
            .max_by_key(|broadcast| broadcast.timestamp)
            .ok_or_else(|| eyre::eyre!("No broadcasts found"))?;

        Ok(target)
    }

    /// Applies the filters and sorts the broadcasts by descending timestamp.
    pub fn filter_and_sort<N: Network>(
        &self,
        broadcasts: Vec<ScriptSequence<N>>,
    ) -> Vec<ScriptSequence<N>> {
        // Apply the filters
        let mut seqs = broadcasts
            .into_iter()
            .filter(|broadcast| {
                if broadcast.chain != self.chain_id {
                    return false;
                }

                broadcast.transactions.iter().any(|tx| self.matches_filters(tx))
            })
            .collect::<Vec<_>>();

        // Sort by descending timestamp
        seqs.sort_by_key(|s| std::cmp::Reverse(s.timestamp));

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
    pub fn into_tx_receipts<N: Network>(
        &self,
        broadcast: ScriptSequence<N>,
    ) -> Vec<(TransactionWithMetadata<N>, N::ReceiptResponse)> {
        let ScriptSequence { transactions, receipts, .. } = broadcast;

        let mut targets: Vec<_> = transactions
            .into_iter()
            .filter(|tx| self.matches_filters(tx))
            .filter_map(|tx| {
                let receipt = receipts
                    .iter()
                    .find(|r| tx.hash.is_some_and(|hash| hash == r.transaction_hash()))?;
                Some((tx, receipt.clone()))
            })
            .collect();

        // Sort by descending block number
        targets.sort_by_key(|t| std::cmp::Reverse(t.1.block_number()));

        targets
    }

    /// Returns the most recent deployment matching a contract name or exact label on this chain.
    ///
    /// Only creation transactions with receipts in the latest matching broadcast are considered.
    /// Contract-name matches retain their existing block-number ordering. If a label matches,
    /// multiple matching deployments in that broadcast are ambiguous and return an error.
    pub fn read_latest_deployment<N: Network>(&self) -> Result<Address>
    where
        N::TxEnvelope: for<'d> Deserialize<'d>,
    {
        let reader = Self {
            tx_type: vec![CallKind::Create, CallKind::Create2],
            match_labels: true,
            ..self.clone()
        };
        let broadcast = reader.read_latest::<N>()?;
        let targets = reader.into_tx_receipts(broadcast);

        if targets.len() > 1
            && targets.iter().any(|(tx, _)| tx.label.as_ref() == Some(&self.contract_name))
        {
            bail!("ambiguous deployment for {} on chain {}", self.contract_name, self.chain_id);
        }

        targets.first().map(|(tx, _)| tx.contract_address.unwrap_or_default()).ok_or_else(|| {
            eyre::eyre!("no deployment found for {} on chain {}", self.contract_name, self.chain_id)
        })
    }
}

/// Deserializes a broadcast sequence, treating receipts without a `type` as legacy like
/// Ethereum's receipt envelope does, so every network accepts the same files.
fn parse_sequence<N: Network>(mut sequence: Value) -> serde_json::Result<ScriptSequence<N>>
where
    N::TxEnvelope: for<'d> Deserialize<'d>,
{
    if let Some(receipts) = sequence.get_mut("receipts").and_then(Value::as_array_mut) {
        for receipt in receipts.iter_mut().filter_map(Value::as_object_mut) {
            if receipt.get("type").is_none_or(Value::is_null) {
                receipt.insert("type".to_string(), "0x0".into());
            }
        }
    }
    serde_json::from_value(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::{AnyNetwork, Ethereum};
    use alloy_primitives::{B256, Bloom};
    use foundry_common::TransactionMaybeSigned;

    fn deployment(address: Address) -> ScriptSequence<Ethereum> {
        let mut tx = TransactionWithMetadata::from_tx_request(TransactionMaybeSigned::Unsigned(
            Default::default(),
        ));
        tx.call_kind = CallKind::Create;
        tx.contract_name = Some("Counter".to_string());
        tx.contract_address = Some(address);
        ScriptSequence { transactions: [tx].into(), timestamp: 1, chain: 1, ..Default::default() }
    }

    #[test]
    fn read_latest_prefers_later_multichain_sequence_on_equal_timestamp() {
        let root = std::env::temp_dir().join(format!("broadcast-reader-{}", std::process::id()));
        let dir = root.join("multi").join("Counter.s.sol-1");
        std::fs::create_dir_all(&dir).unwrap();
        let (older, newer) = (Address::with_last_byte(1), Address::with_last_byte(2));
        fs::write_json_file(
            &dir.join("deploy.json"),
            &serde_json::json!({ "deployments": [deployment(older), deployment(newer)] }),
        )
        .unwrap();

        let latest = BroadcastReader::new("Counter".to_string(), 1, &root)
            .unwrap()
            .read_latest::<Ethereum>()
            .unwrap();
        assert_eq!(latest.transactions[0].contract_address, Some(newer));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_receipts_without_type_as_legacy() {
        let root = std::env::temp_dir().join(format!("broadcast-typeless-{}", std::process::id()));
        let hash = B256::with_last_byte(1);
        let sequence = |address: Address, ty: Option<serde_json::Value>| {
            let mut sequence = deployment(address);
            sequence.transactions[0].hash = Some(hash);
            let mut sequence = serde_json::to_value(sequence).unwrap();
            let mut receipt = serde_json::json!({
                "status": "0x1",
                "cumulativeGasUsed": "0x5208",
                "logs": [],
                "logsBloom": Bloom::ZERO,
                "transactionHash": hash,
                "transactionIndex": "0x0",
                "blockHash": B256::ZERO,
                "blockNumber": "0x7",
                "gasUsed": "0x5208",
                "effectiveGasPrice": "0x1",
                "from": Address::ZERO,
                "to": null,
                "contractAddress": address,
            });
            if let Some(ty) = ty {
                receipt["type"] = ty;
            }
            sequence["receipts"] = serde_json::json!([receipt]);
            sequence
        };
        let dir = root.join("Counter.s.sol").join("1");
        std::fs::create_dir_all(&dir).unwrap();
        let reader = BroadcastReader::new("Counter".to_string(), 1, &root).unwrap();
        fs::write_json_file(&dir.join("run-1.json"), &sequence(Address::with_last_byte(1), None))
            .unwrap();
        let latest = reader.read_latest::<AnyNetwork>().unwrap();
        let receipts = reader.into_tx_receipts(latest);
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].1.block_number(), Some(7));
        assert!(receipts[0].1.status());
        std::fs::remove_dir_all(&root).unwrap();

        let dir = root.join("multi").join("Counter.s.sol-1");
        std::fs::create_dir_all(&dir).unwrap();
        let deployments = [
            sequence(Address::with_last_byte(2), Some(serde_json::Value::Null)),
            sequence(Address::with_last_byte(3), Some("0x2".into())),
        ];
        fs::write_json_file(
            &dir.join("deploy.json"),
            &serde_json::json!({ "deployments": deployments }),
        )
        .unwrap();
        assert_eq!(reader.read::<AnyNetwork>().unwrap().len(), 2);

        std::fs::remove_dir_all(root).unwrap();
    }

    fn confirmed_deployment(
        address: Address,
        label: Option<&str>,
        block: u64,
    ) -> ScriptSequence<Ethereum> {
        let mut sequence = deployment(address);
        let hash = B256::with_last_byte(address[19]);
        sequence.transactions[0].hash = Some(hash);
        sequence.transactions[0].label = label.map(str::to_string);
        sequence.receipts.push(
            serde_json::from_value(serde_json::json!({
                "type": "0x0",
                "status": "0x1",
                "cumulativeGasUsed": "0x5208",
                "logs": [],
                "logsBloom": Bloom::ZERO,
                "transactionHash": hash,
                "transactionIndex": "0x0",
                "blockHash": B256::ZERO,
                "blockNumber": format!("{block:#x}"),
                "gasUsed": "0x5208",
                "effectiveGasPrice": "0x1",
                "from": Address::ZERO,
                "to": null,
                "contractAddress": address,
            }))
            .unwrap(),
        );
        sequence
    }

    fn deployment_lookup(
        test_name: &str,
        name: &str,
        broadcasts: &[ScriptSequence<Ethereum>],
    ) -> BroadcastReader {
        let root = std::env::temp_dir().join(format!("{test_name}-{}", std::process::id()));
        let dir = root.join("Counter.s.sol").join("1");
        std::fs::create_dir_all(&dir).unwrap();
        for (index, broadcast) in broadcasts.iter().enumerate() {
            fs::write_json_file(&dir.join(format!("run-{index}.json")), broadcast).unwrap();
        }
        BroadcastReader::new(name.to_string(), 1, &root).unwrap()
    }

    #[test]
    fn deployment_lookup_resolves_exact_labels() {
        let first = Address::with_last_byte(1);
        let second = Address::with_last_byte(2);
        let mut sequence = confirmed_deployment(first, Some("UsdcPool"), 1);
        let other = confirmed_deployment(second, Some("WethPool"), 2);
        sequence.transactions.extend(other.transactions);
        sequence.receipts.extend(other.receipts);
        let reader = deployment_lookup("deployment-label", "UsdcPool", &[sequence]);

        assert_eq!(reader.read_latest_deployment::<Ethereum>().unwrap(), first);
        assert_eq!(reader.read_latest_deployment::<AnyNetwork>().unwrap(), first);
        assert!(reader.read::<Ethereum>().unwrap().is_empty());
        assert!(
            BroadcastReader::new("usdcpool".to_string(), 1, &reader.broadcast_path)
                .unwrap()
                .read_latest_deployment::<Ethereum>()
                .is_err()
        );

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_preserves_legacy_contract_names() {
        let first = Address::with_last_byte(1);
        let second = Address::with_last_byte(2);
        let mut sequence = confirmed_deployment(first, None, 1);
        let other = confirmed_deployment(second, None, 2);
        sequence.transactions.extend(other.transactions);
        sequence.receipts.extend(other.receipts);
        let reader = deployment_lookup("deployment-legacy", "Counter", &[sequence]);

        assert_eq!(reader.read_latest_deployment::<Ethereum>().unwrap(), second);

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_rejects_duplicate_labels() {
        let mut sequence = confirmed_deployment(Address::with_last_byte(1), Some("Pool"), 1);
        let other = confirmed_deployment(Address::with_last_byte(2), Some("Pool"), 2);
        sequence.transactions.extend(other.transactions);
        sequence.receipts.extend(other.receipts);
        let reader = deployment_lookup("deployment-duplicate-label", "Pool", &[sequence]);

        assert_eq!(
            reader.read_latest_deployment::<Ethereum>().unwrap_err().to_string(),
            "ambiguous deployment for Pool on chain 1"
        );

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_rejects_label_contract_name_collisions() {
        let mut sequence = confirmed_deployment(Address::with_last_byte(1), Some("Counter"), 1);
        let other = confirmed_deployment(Address::with_last_byte(2), None, 2);
        sequence.transactions.extend(other.transactions);
        sequence.receipts.extend(other.receipts);
        let reader = deployment_lookup("deployment-name-collision", "Counter", &[sequence]);

        assert_eq!(
            reader.read_latest_deployment::<Ethereum>().unwrap_err().to_string(),
            "ambiguous deployment for Counter on chain 1"
        );

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_allows_a_label_equal_to_its_own_contract_name() {
        let address = Address::with_last_byte(1);
        let sequence = confirmed_deployment(address, Some("Counter"), 1);
        let reader = deployment_lookup("deployment-own-name", "Counter", &[sequence]);

        assert_eq!(reader.read_latest_deployment::<Ethereum>().unwrap(), address);

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_uses_the_latest_matching_run_on_the_selected_chain() {
        let older = confirmed_deployment(Address::with_last_byte(1), Some("Pool"), 1);
        let address = Address::with_last_byte(2);
        let mut newer = confirmed_deployment(address, Some("Pool"), 2);
        newer.timestamp = 2;
        newer.transactions[0].call_kind = CallKind::Create2;
        let mut other_chain = confirmed_deployment(Address::with_last_byte(3), Some("Pool"), 3);
        other_chain.timestamp = 3;
        other_chain.chain = 2;
        let reader =
            deployment_lookup("deployment-latest-label", "Pool", &[older, newer, other_chain]);

        assert_eq!(reader.read_latest_deployment::<Ethereum>().unwrap(), address);

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_ignores_calls_and_transactions_without_receipts() {
        let address = Address::with_last_byte(1);
        let mut sequence = confirmed_deployment(address, Some("Pool"), 1);
        let mut call = confirmed_deployment(Address::with_last_byte(2), Some("Pool"), 2);
        call.transactions[0].call_kind = CallKind::Call;
        sequence.transactions.extend(call.transactions);
        sequence.receipts.extend(call.receipts);
        let pending = confirmed_deployment(Address::with_last_byte(3), Some("Pool"), 3);
        sequence.transactions.extend(pending.transactions);
        let reader = deployment_lookup("deployment-label-filters", "Pool", &[sequence]);

        assert_eq!(reader.read_latest_deployment::<Ethereum>().unwrap(), address);

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }
}
