use crate::{ScriptSequence, TransactionWithMetadata};
use alloy_network::{Network, ReceiptResponse};
use alloy_primitives::Address;
use evm2_inspectors::tracing::types::CallKind;
use eyre::{Result, bail};
use foundry_common::{errors::FsPathError, fs};
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
        })
    }

    /// Set the transaction type to filter by.
    pub fn with_tx_type(mut self, tx_type: CallKind) -> Self {
        self.tx_type.push(tx_type);
        self
    }

    fn matches_filters<N: Network>(&self, tx: &TransactionWithMetadata<N>) -> bool {
        let name_filter = tx.contract_name.as_ref().is_some_and(|cn| *cn == self.contract_name);
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
        Ok(self.filter_and_sort(self.read_all()?))
    }

    /// Reads every broadcast in the broadcast directory without applying the filters.
    fn read_all<N: Network>(&self) -> eyre::Result<Vec<ScriptSequence<N>>>
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

    /// Returns the most recent deployment whose contract name or exact `vm.label` is the
    /// contract name of this reader.
    ///
    /// Contract names resolve as before: the highest-block confirmed CREATE or CREATE2 in the
    /// latest broadcast with a matching transaction. Labels resolve in the latest broadcast with a
    /// confirmed labeled deployment, including contracts created by other contracts, and must name
    /// a single address there. A label equal to its deployment's own contract name is ignored.
    /// When a broadcast has a matching contract name, the contract name decides, and a label that
    /// resolves to a different deployment is an error.
    pub fn read_latest_deployment<N: Network>(&self) -> Result<Address>
    where
        N::TxEnvelope: for<'d> Deserialize<'d>,
    {
        let reader = Self { tx_type: vec![CallKind::Create, CallKind::Create2], ..self.clone() };
        let broadcasts = reader.read_all::<N>()?;
        let query = Some(self.contract_name.as_str());
        let label = broadcasts
            .iter()
            .filter(|broadcast| broadcast.chain == self.chain_id)
            .filter_map(|broadcast| {
                let addresses = broadcast
                    .transactions
                    .iter()
                    .filter(|tx| {
                        tx.hash.is_some_and(|hash| {
                            broadcast.receipts.iter().any(|r| r.transaction_hash() == hash)
                        })
                    })
                    .flat_map(|tx| {
                        let created = tx.call_kind.is_any_create().then_some((
                            tx.contract_name.as_deref(),
                            tx.label.as_deref(),
                            tx.contract_address,
                        ));
                        created.into_iter().chain(tx.additional_contracts.iter().map(|contract| {
                            (
                                contract.contract_name.as_deref(),
                                contract.label.as_deref(),
                                Some(contract.address),
                            )
                        }))
                    })
                    .filter(|&(name, label, _)| label == query && name != query)
                    .filter_map(|(.., address)| address)
                    .collect::<Vec<_>>();
                (!addresses.is_empty()).then_some((broadcast.timestamp, addresses))
            })
            .max_by_key(|(timestamp, _)| *timestamp)
            .map(|(_, addresses)| {
                let [address] = addresses[..] else {
                    let addresses = addresses.iter().map(Address::to_string).collect::<Vec<_>>();
                    bail!(
                        "label {} matches multiple deployments on chain {}: {}",
                        self.contract_name,
                        self.chain_id,
                        addresses.join(", ")
                    );
                };
                Ok(address)
            })
            .transpose()?;
        let Some(broadcast) = reader
            .filter_and_sort(broadcasts)
            .into_iter()
            .max_by_key(|broadcast| broadcast.timestamp)
        else {
            return label.ok_or_else(|| eyre::eyre!("No broadcasts found"));
        };
        let targets = reader.into_tx_receipts(broadcast);
        let Some((tx, _)) = targets.first() else {
            bail!("no deployment found for {} on chain {}", self.contract_name, self.chain_id);
        };
        let name = tx.contract_address.unwrap_or_default();

        if let Some(label) = label
            && label != name
        {
            bail!(
                "{} is the label of {label} and the contract name of {name} on chain {}",
                self.contract_name,
                self.chain_id
            );
        }
        Ok(name)
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
    use crate::AdditionalContract;
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

    fn merge(
        sequences: impl IntoIterator<Item = ScriptSequence<Ethereum>>,
    ) -> ScriptSequence<Ethereum> {
        let mut merged = ScriptSequence { timestamp: 1, chain: 1, ..Default::default() };
        for sequence in sequences {
            merged.transactions.extend(sequence.transactions);
            merged.receipts.extend(sequence.receipts);
        }
        merged
    }

    fn deployment_lookup(
        test_name: &str,
        name: &str,
        broadcasts: &[ScriptSequence<Ethereum>],
    ) -> BroadcastReader {
        let root = std::env::temp_dir().join(format!("{test_name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("Counter.s.sol").join("1");
        std::fs::create_dir_all(&dir).unwrap();
        for (index, broadcast) in broadcasts.iter().enumerate() {
            fs::write_json_file(&dir.join(format!("run-{index}.json")), broadcast).unwrap();
        }
        BroadcastReader::new(name.to_string(), 1, &root).unwrap()
    }

    fn lookup(reader: &BroadcastReader) -> Result<Address, String> {
        reader.read_latest_deployment::<AnyNetwork>().map_err(|err| err.to_string())
    }

    #[test]
    fn deployment_lookup_resolves_exact_labels() {
        let (usdc, weth) = (Address::with_last_byte(1), Address::with_last_byte(2));
        let sequence = merge([
            confirmed_deployment(usdc, Some("UsdcPool"), 1),
            confirmed_deployment(weth, Some("WethPool"), 2),
        ]);
        let reader = deployment_lookup("deployment-label", "UsdcPool", &[sequence]);

        assert_eq!(lookup(&reader), Ok(usdc));
        assert!(reader.read::<Ethereum>().unwrap().is_empty());
        let lowercase = BroadcastReader::new("usdcpool".to_string(), 1, &reader.broadcast_path);
        assert!(lookup(&lowercase.unwrap()).is_err());

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_preserves_contract_names_with_own_name_labels() {
        let (labeled, latest) = (Address::with_last_byte(1), Address::with_last_byte(2));
        let unlabeled =
            merge([confirmed_deployment(labeled, None, 1), confirmed_deployment(latest, None, 2)]);
        let reader = deployment_lookup("deployment-legacy", "Counter", &[unlabeled]);
        assert_eq!(lookup(&reader), Ok(latest));
        std::fs::remove_dir_all(reader.broadcast_path).unwrap();

        let labeled = merge([
            confirmed_deployment(labeled, Some("Counter"), 1),
            confirmed_deployment(latest, Some("Counter"), 2),
        ]);
        let reader = deployment_lookup("deployment-own-name", "Counter", &[labeled]);
        assert_eq!(lookup(&reader), Ok(latest));
        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_rejects_duplicate_labels() {
        let sequence = merge([
            confirmed_deployment(Address::with_last_byte(1), Some("Pool"), 1),
            confirmed_deployment(Address::with_last_byte(2), Some("Pool"), 2),
        ]);
        let reader = deployment_lookup("deployment-duplicate-label", "Pool", &[sequence]);

        assert_eq!(
            lookup(&reader),
            Err("label Pool matches multiple deployments on chain 1: \
                 0x0000000000000000000000000000000000000001, \
                 0x0000000000000000000000000000000000000002"
                .to_string())
        );

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_rejects_labels_naming_another_contract_in_any_run() {
        let (labeled, named) = (Address::with_last_byte(1), Address::with_last_byte(2));
        let label = confirmed_deployment(labeled, Some("Token"), 1);
        let mut name = confirmed_deployment(named, None, 2);
        name.transactions[0].contract_name = Some("Token".to_string());
        let error = Err("Token is the label of 0x0000000000000000000000000000000000000001 and \
                         the contract name of 0x0000000000000000000000000000000000000002 on chain 1"
            .to_string());

        for (test_name, older, mut newer) in [
            ("deployment-newer-name", label.clone(), name.clone()),
            ("deployment-newer-label", name.clone(), label.clone()),
        ] {
            newer.timestamp = 2;
            let reader = deployment_lookup(test_name, "Token", &[older, newer]);
            assert_eq!(lookup(&reader), error);
            std::fs::remove_dir_all(reader.broadcast_path).unwrap();
        }

        let mut unconfirmed = name;
        unconfirmed.receipts.clear();
        let reader =
            deployment_lookup("deployment-unconfirmed-name", "Token", &[label, unconfirmed]);
        assert_eq!(lookup(&reader), Err("no deployment found for Token on chain 1".to_string()));
        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_uses_the_latest_labeled_run_on_the_selected_chain() {
        let older = confirmed_deployment(Address::with_last_byte(1), Some("Pool"), 1);
        let address = Address::with_last_byte(2);
        let mut newer = confirmed_deployment(address, Some("Pool"), 2);
        newer.timestamp = 2;
        newer.transactions[0].call_kind = CallKind::Create2;
        let mut other_chain = confirmed_deployment(Address::with_last_byte(3), Some("Pool"), 3);
        other_chain.timestamp = 3;
        other_chain.chain = 2;
        let mut unconfirmed = deployment(Address::with_last_byte(4));
        unconfirmed.transactions[0].label = Some("Pool".to_string());
        unconfirmed.timestamp = 4;
        let reader = deployment_lookup(
            "deployment-recent-label",
            "Pool",
            &[older, newer, other_chain, unconfirmed],
        );

        assert_eq!(lookup(&reader), Ok(address));

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }

    #[test]
    fn deployment_lookup_resolves_labeled_factory_contracts() {
        let pool = Address::with_last_byte(2);
        let mut sequence = confirmed_deployment(Address::with_last_byte(1), None, 1);
        sequence.transactions[0].call_kind = CallKind::Call;
        sequence.transactions[0].label = Some("Pool".to_string());
        sequence.transactions[0].additional_contracts.push(AdditionalContract {
            contract_name: Some("Pool".to_string()),
            address: Address::with_last_byte(3),
            label: Some("Pool".to_string()),
            ..Default::default()
        });
        sequence.transactions[0].additional_contracts.push(AdditionalContract {
            contract_name: Some("Pool".to_string()),
            address: pool,
            label: Some("UsdcPool".to_string()),
            ..Default::default()
        });
        let pending = confirmed_deployment(Address::with_last_byte(4), Some("UsdcPool"), 3);
        let mut sequence = merge([sequence]);
        sequence.transactions.extend(pending.transactions);
        let reader = deployment_lookup("deployment-factory-label", "UsdcPool", &[sequence]);

        assert_eq!(lookup(&reader), Ok(pool));
        let own_name = BroadcastReader::new("Pool".to_string(), 1, &reader.broadcast_path);
        assert_eq!(lookup(&own_name.unwrap()), Err("No broadcasts found".to_string()));

        std::fs::remove_dir_all(reader.broadcast_path).unwrap();
    }
}
