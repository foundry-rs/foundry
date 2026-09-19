//! Freeze a stratified panel from block metadata before observing BAL availability.

use super::{BlockInput, Case, Manifest, TargetInput, digest, endpoint, new_output, write_json};
use crate::CaptureArgs;
use alloy_primitives::B256;
use eyre::{Result, ensure};
use foundry_config::FoundryHardfork;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};

pub struct Rpc {
    client: reqwest::Client,
    endpoint: String,
}

impl Rpc {
    pub fn new(endpoint: &str) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()?,
            endpoint: endpoint.into(),
        })
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await
            .map_err(|_| eyre::eyre!("RPC transport failed for {method}"))?;
        ensure!(
            response.status().is_success(),
            "RPC HTTP failure for {method}: {}",
            response.status()
        );
        let body: Value =
            response.json().await.map_err(|_| eyre::eyre!("invalid RPC response for {method}"))?;
        ensure!(
            body.get("error").is_none(),
            "RPC error for {method} (code {})",
            body["error"]["code"]
        );
        ensure!(body.get("result").is_some(), "missing RPC result for {method}");
        Ok(body["result"].clone())
    }
}

pub fn quantity(value: &Value) -> Result<u64> {
    let s = value.as_str().ok_or_else(|| eyre::eyre!("expected hex quantity"))?;
    Ok(u64::from_str_radix(
        s.strip_prefix("0x").ok_or_else(|| eyre::eyre!("expected hex prefix"))?,
        16,
    )?)
}

fn hash(value: &Value) -> Result<B256> {
    Ok(value.as_str().ok_or_else(|| eyre::eyre!("missing block or transaction hash"))?.parse()?)
}

pub fn positions(count: usize) -> BTreeMap<usize, Vec<String>> {
    let mut selected = BTreeMap::<usize, Vec<String>>::new();
    if count > 0 {
        for (index, label) in [(0, "first"), ((count - 1) / 2, "middle"), (count - 1, "last")] {
            selected.entry(index).or_default().push(label.into());
        }
    }
    selected
}

pub fn save_capture(root: &Path, value: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let hash = digest(&bytes);
    std::fs::write(root.join("capture").join(format!("{hash}.json")), bytes)?;
    Ok(hash)
}

pub async fn capture(args: CaptureArgs) -> Result<()> {
    let count = args.blocks;
    ensure!(count > 0, "block count must be positive");
    let candidates = args
        .candidate_blocks
        .unwrap_or(count.checked_mul(10).ok_or_else(|| eyre::eyre!("candidate count overflow"))?);
    ensure!(candidates >= count, "candidate interval must cover requested sample count");
    ensure!(!args.endpoint_label.contains("://"), "endpoint label must not contain a URL");
    let upstream = endpoint(&args.endpoint)?;
    let rpc = Rpc::new(&upstream)?;
    let chain_id = quantity(&rpc.call("eth_chainId", json!([])).await?)?;
    let client_version =
        rpc.call("web3_clientVersion", json!([])).await?.as_str().unwrap_or("unknown").to_owned();
    let finalized = rpc.call("eth_getBlockByNumber", json!(["finalized", false])).await?;
    let finalized_number = quantity(&finalized["number"])?;
    let end = args.end_block.unwrap_or(finalized_number);
    ensure!(end <= finalized_number, "candidate interval must be finalized");
    let start = end
        .checked_sub(candidates as u64 - 1)
        .ok_or_else(|| eyre::eyre!("candidate interval extends before genesis"))?;
    new_output(&args.output_dir)?;
    let mut metadata = Vec::new();
    let mut capture_failures = Vec::new();
    for number in start..=end {
        match rpc.call("eth_getBlockByNumber", json!([format!("0x{number:x}"), false])).await {
            Ok(block) if block.get("transactions").and_then(Value::as_array).is_some() => {
                metadata.push(block)
            }
            _ => capture_failures.push(json!({"number":number,"status":"metadata_unavailable"})),
        }
    }
    let metadata_hash =
        save_capture(&args.output_dir, &json!({"metadata":metadata,"failures":capture_failures}))?;
    let mut sizes = metadata
        .iter()
        .filter_map(|b| b["transactions"].as_array().map(Vec::len))
        .filter(|n| *n > 0)
        .collect::<Vec<_>>();
    sizes.sort_unstable();
    ensure!(
        sizes.len() >= count,
        "not enough nonempty metadata blocks; failed candidates retained in capture"
    );
    let large_threshold = sizes[(sizes.len() * 9 / 10).min(sizes.len() - 1)];
    let mut strata = BTreeMap::<String, Vec<Value>>::new();
    for block in metadata {
        let n = block["transactions"].as_array().map_or(0, Vec::len);
        if n > 0 {
            let timestamp = quantity(&block["timestamp"])?;
            let hardfork = FoundryHardfork::from_chain_and_timestamp(chain_id, timestamp)
                .map_or_else(|| "unknown".to_string(), |hf| format!("{hf:?}"));
            let size = if n >= large_threshold { "large" } else { "ordinary" };
            strata.entry(format!("{hardfork}/{size}")).or_default().push(block);
        }
    }
    ensure!(strata.len() <= count, "requested sample too small to represent all metadata strata");
    for blocks in strata.values_mut() {
        blocks.sort_by_cached_key(|block| {
            digest(format!("{}:{}", args.seed, block["hash"]).as_bytes())
        });
    }
    let mut selected = Vec::new();
    let mut allocations = BTreeMap::<String, usize>::new();
    let mut depth = 0;
    while selected.len() < count {
        for (stratum, blocks) in &strata {
            if selected.len() < count
                && let Some(block) = blocks.get(depth)
            {
                selected.push((stratum.clone(), block.clone()));
                *allocations.entry(stratum.clone()).or_default() += 1;
            }
        }
        depth += 1;
    }
    // This file freezes selected hashes and weights before receipts or any BAL probe.
    write_json(
        &args.output_dir.join("selection.json"),
        &json!({"schema_version":1,"seed":args.seed,
        "start_block":start,"end_block":end,"candidate_metadata_sha256":metadata_hash,
        "large_transaction_count_threshold":large_threshold,"selection":selected,
        "allocations":allocations,"selection_algorithm":"sha256_seed_hash_stratified_round_robin_v1"}),
    )?;
    let mut blocks = Vec::new();
    let mut cases = Vec::new();
    let mut raw_hashes = Vec::new();
    for (stratum, selected_block) in selected {
        let block_hash = hash(&selected_block["hash"])?;
        let block_number = quantity(&selected_block["number"])?;
        let parent_hash = hash(&selected_block["parentHash"])?;
        let txs =
            selected_block["transactions"].as_array().expect("validated metadata transactions");
        let mut targets = Vec::new();
        for (index, positions) in positions(txs.len()) {
            let transaction_hash = hash(&txs[index])?;
            let id = format!("b{block_number}-t{index}");
            targets.push(TargetInput { id: id.clone(), tx_hash: transaction_hash, index });
            match rpc.call("eth_getTransactionReceipt", json!([transaction_hash])).await {
                Ok(receipt)
                    if receipt["blockHash"] == json!(block_hash)
                        && receipt["transactionIndex"] == json!(format!("0x{index:x}")) =>
                {
                    raw_hashes.push(save_capture(&args.output_dir, &receipt)?);
                    cases.push(Case {
                        id,
                        transaction_hash,
                        block_hash,
                        index,
                        positions,
                        stratum: stratum.clone(),
                        expected_receipt_gas: Some(quantity(&receipt["gasUsed"])?),
                        expected_receipt_status: Some(quantity(&receipt["status"])? != 0),
                        capture_error: None,
                        bal_response: None,
                        fault: None,
                    });
                }
                _ => {
                    capture_failures.push(json!({"target":id,"status":"receipt_unavailable"}));
                    cases.push(Case {
                        id,
                        transaction_hash,
                        block_hash,
                        index,
                        positions,
                        stratum: stratum.clone(),
                        expected_receipt_gas: None,
                        expected_receipt_status: None,
                        capture_error: Some("receipt_unavailable".into()),
                        bal_response: None,
                        fault: None,
                    });
                }
            }
        }
        raw_hashes.push(save_capture(&args.output_dir, &selected_block)?);
        blocks.push(BlockInput { block_hash, block_number, parent_hash, targets, stratum });
    }
    let manifest = Manifest {
        schema_version: 1,
        endpoint_label: args.endpoint_label,
        chain_id,
        client_version,
        seed: args.seed,
        blocks,
        cases,
        source_context: json!({"kind":"live_http","server_bal_source":"unknown","captured_at":chrono::Utc::now(),
            "finalized_hash":finalized["hash"],"finalized_number":finalized_number,"candidate_start":start,"candidate_end":end,
            "large_transaction_count_threshold":large_threshold,"candidate_metadata_sha256":metadata_hash,"raw_sha256":raw_hashes,
            "failures":capture_failures,"purpose":"performance_panel",
            "unrepresented_periods":"hardfork periods outside the frozen candidate interval are not sampled"}),
    };
    write_json(&args.output_dir.join("manifest.json"), &manifest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::positions;

    #[test]
    fn positions_deduplicate_single_and_two_transaction_blocks() {
        assert!(positions(0).is_empty());
        assert_eq!(positions(1)[&0], ["first", "middle", "last"]);
        assert_eq!(positions(2).len(), 2);
        assert_eq!(positions(100).keys().copied().collect::<Vec<_>>(), [0, 49, 99]);
    }
}
