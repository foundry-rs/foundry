use super::logs::LogQueryArgs;
use crate::{
    Cast, MAX_CONCURRENT_RPC_REQUESTS,
    traces::{
        CallTraceDecoderBuilder,
        identifier::{ExternalIdentifier, SignaturesIdentifier},
    },
};
use alloy_primitives::{Address, B256, Bytes, TxHash};
use alloy_provider::Provider;
use alloy_rpc_types::Log;
use clap::{ArgGroup, Parser};
use eyre::Result;
use foundry_cli::{
    json::print_json_object,
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, LoadConfig},
};
use foundry_common::shell;
use foundry_config::{Chain, Config};
use futures::StreamExt;
use serde::Serialize;
use std::{collections::BTreeSet, fmt::Write as _};

foundry_config::impl_figment_convert!(EventsArgs, etherscan, rpc);

/// CLI arguments for `cast events`.
#[derive(Debug, Parser)]
#[command(group(
    ArgGroup::new("event_source")
        .required(true)
        .multiple(true)
        .args(["tx_hash", "address", "from_block", "to_block", "sig_or_topic"])
))]
pub struct EventsArgs {
    /// Get events emitted by this transaction.
    #[arg(
        long,
        alias = "txhash",
        value_name = "TX_HASH",
        conflicts_with_all = [
            "from_block",
            "to_block",
            "address",
            "sig_or_topic",
            "topics_or_args",
            "query_size"
        ]
    )]
    tx_hash: Option<TxHash>,

    #[command(flatten)]
    query: LogQueryArgs,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl EventsArgs {
    pub async fn run(self) -> Result<()> {
        let mut config = self.load_config()?;
        let Self { tx_hash, query, etherscan: _, rpc: _ } = self;
        let provider = utils::get_provider(&config)?;
        let chain_id = provider.get_chain_id().await?;
        let (rpc_chain, explorer_chain) = resolve_chains(config.chain, Chain::from(chain_id));
        config.chain = Some(rpc_chain);

        let cast = Cast::new(&provider);
        let logs = if let Some(tx_hash) = tx_hash {
            cast.get_transaction_logs(tx_hash).await?
        } else {
            let (filter, query_size) = query.resolve(&provider).await?;
            match query_size {
                Some(chunk_size) => cast.get_logs_chunked(&filter, chunk_size).await?,
                None => cast.get_logs(&filter).await?,
            }
        };

        let events = decode_logs(logs, &config, explorer_chain).await?;
        if shell::is_json() {
            print_json_object(events)?;
        } else {
            // Bypass the shell verbosity layer so `--quiet` does not suppress the primary result.
            let mut shell = shell::Shell::get();
            let out = shell.out();
            write!(out, "{}", format_events(&events))?;
            out.flush()?;
        }
        Ok(())
    }
}

fn resolve_chains(configured_chain: Option<Chain>, rpc_chain: Chain) -> (Chain, Chain) {
    (rpc_chain, configured_chain.unwrap_or(rpc_chain))
}

async fn decode_logs(
    logs: Vec<Log>,
    config: &Config,
    explorer_chain: Chain,
) -> Result<Vec<EventOutput>> {
    let signature_identifier = SignaturesIdentifier::from_config(config)?;
    let mut builder = CallTraceDecoderBuilder::new()
        .with_signature_identifier(signature_identifier)
        .with_networks(config.networks)
        .with_chain_id(config.chain.map(|chain| chain.id()));

    if let Some(mut identifier) = ExternalIdentifier::new(config, Some(explorer_chain))? {
        let addresses =
            logs.iter().map(Log::address).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        for (address, result) in identifier.get_abis(&addresses).await {
            match result {
                Ok(abis) => {
                    for abi in abis {
                        builder = builder.with_address_abi(address, &abi);
                    }
                }
                Err(err) => sh_warn!("Failed to fetch ABI for {address}: {err}")?,
            }
        }
    }

    let decoder = builder.build();
    Ok(futures::stream::iter(logs)
        .map(|log| async {
            let decoded = decoder.decode_event_with_address(log.address(), log.data()).await;
            EventOutput::new(log, decoded.name, decoded.params)
        })
        .buffered(MAX_CONCURRENT_RPC_REQUESTS)
        .collect()
        .await)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventOutput {
    address: Address,
    block_hash: Option<B256>,
    block_number: Option<u64>,
    block_timestamp: Option<u64>,
    transaction_hash: Option<TxHash>,
    transaction_index: Option<u64>,
    log_index: Option<u64>,
    removed: bool,
    event: Option<String>,
    params: Option<Vec<EventParam>>,
    topics: Vec<B256>,
    data: Bytes,
}

impl EventOutput {
    fn new(log: Log, event: Option<String>, params: Option<Vec<(String, String)>>) -> Self {
        let params = params.map(|params| {
            params
                .into_iter()
                .enumerate()
                .map(|(index, (name, value))| EventParam {
                    name: if name.is_empty() { format!("param{index}") } else { name },
                    value,
                })
                .collect()
        });
        Self {
            address: log.address(),
            block_hash: log.block_hash,
            block_number: log.block_number,
            block_timestamp: log.block_timestamp,
            transaction_hash: log.transaction_hash,
            transaction_index: log.transaction_index,
            log_index: log.log_index,
            removed: log.removed,
            event,
            params,
            topics: log.topics().to_vec(),
            data: log.data().data.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct EventParam {
    name: String,
    value: String,
}

/// Formats decoded and raw events for human-readable output.
///
/// # Example
///
/// ```text
/// [block 1, tx 0xabc..., log 0] 0x123...::Transfer(from: 0x456..., value: 1)
/// 0x789...
///   topic 0: 0xdef...
///   data: 0x
/// ```
fn format_events(events: &[EventOutput]) -> String {
    let mut output = String::new();
    for event in events {
        if event.block_number.is_some()
            || event.transaction_hash.is_some()
            || event.log_index.is_some()
        {
            output.push('[');
            if let Some(block_number) = event.block_number {
                let _ = write!(output, "block {block_number}");
            }
            if let Some(transaction_hash) = event.transaction_hash {
                if event.block_number.is_some() {
                    output.push_str(", ");
                }
                let _ = write!(output, "tx {transaction_hash}");
            }
            if let Some(log_index) = event.log_index {
                if event.block_number.is_some() || event.transaction_hash.is_some() {
                    output.push_str(", ");
                }
                let _ = write!(output, "log {log_index}");
            }
            output.push_str("] ");
        }
        if let Some(name) = &event.event {
            let _ = write!(output, "{}::{name}(", event.address);
            if let Some(params) = &event.params {
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    let _ = write!(output, "{}: {}", param.name, param.value);
                }
            }
            output.push_str(")\n");
        } else {
            let _ = writeln!(output, "{}", event.address);
            for (index, topic) in event.topics.iter().enumerate() {
                let _ = writeln!(output, "  topic {index}: {topic}");
            }
            let _ = writeln!(output, "  data: {}", event.data);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_abi::{Event, JsonAbi};
    use alloy_network::AnyNetwork;
    use alloy_primitives::{LogData, U256};
    use alloy_provider::{ProviderBuilder, mock::Asserter};
    use alloy_sol_types::SolValue;

    #[test]
    fn requires_event_source() {
        assert!(EventsArgs::try_parse_from(["events"]).is_err());
    }

    #[test]
    fn transaction_and_filter_modes_conflict() {
        assert!(
            EventsArgs::try_parse_from([
                "events",
                "--tx-hash",
                &TxHash::ZERO.to_string(),
                "--address",
                &Address::ZERO.to_string(),
            ])
            .is_err()
        );
    }

    #[test]
    fn accepts_transaction_and_filter_modes_separately() {
        assert!(
            EventsArgs::try_parse_from(["events", "--tx-hash", &TxHash::ZERO.to_string()]).is_ok()
        );
        assert!(
            EventsArgs::try_parse_from(["events", "--txhash", &TxHash::ZERO.to_string()]).is_ok()
        );
        assert!(
            EventsArgs::try_parse_from([
                "events",
                "--address",
                &Address::ZERO.to_string(),
                "--from-block",
                "1",
                "--to-block",
                "2",
            ])
            .is_ok()
        );
    }

    #[test]
    fn configured_chain_controls_explorer_lookup() {
        let rpc_chain = Chain::from(31337);
        let (decoder_chain, explorer_chain) = resolve_chains(Some(Chain::mainnet()), rpc_chain);
        assert_eq!(decoder_chain, rpc_chain);
        assert_eq!(explorer_chain, Chain::mainnet());

        let (_, explorer_chain) = resolve_chains(None, rpc_chain);
        assert_eq!(explorer_chain, rpc_chain);
    }

    #[tokio::test]
    async fn fetches_receipt_logs_and_reports_missing_receipts() {
        let tx_hash = TxHash::repeat_byte(0x44);
        let log_address = Address::repeat_byte(0xaa);
        let receipt = serde_json::json!({
            "type": "0x2",
            "status": "0x1",
            "cumulativeGasUsed": "0x5208",
            "logs": [{
                "address": log_address,
                "topics": [],
                "data": "0x",
                "blockNumber": "0x7",
                "transactionHash": tx_hash,
                "transactionIndex": "0x0",
                "blockHash": B256::repeat_byte(0x33),
                "logIndex": "0x3",
                "removed": false
            }],
            "transactionHash": tx_hash,
            "transactionIndex": "0x0",
            "blockHash": B256::repeat_byte(0x33),
            "blockNumber": "0x7",
            "logsBloom": format!("0x{}", "0".repeat(512)),
            "gasUsed": "0x5208",
            "effectiveGasPrice": "0x1",
            "from": Address::ZERO,
            "to": Address::ZERO,
            "contractAddress": null
        });
        let asserter = Asserter::new();
        asserter.push_success(&receipt);
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter.clone());
        let cast = Cast::new(&provider);

        let logs = cast.get_transaction_logs(tx_hash).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].address(), log_address);
        assert_eq!(logs[0].block_number, Some(7));
        assert_eq!(logs[0].log_index, Some(3));

        let missing: Option<serde_json::Value> = None;
        asserter.push_success(&missing);
        let err = cast.get_transaction_logs(tx_hash).await.unwrap_err();
        assert!(err.to_string().contains("tx receipt not found"));
    }

    #[tokio::test]
    async fn fetches_filtered_logs_in_chunk_order() {
        let asserter = Asserter::new();
        asserter
            .push_success(&vec![Log::<LogData> { block_number: Some(1), ..Default::default() }]);
        asserter
            .push_success(&vec![Log::<LogData> { block_number: Some(2), ..Default::default() }]);
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter);
        let EventsArgs { query, .. } = EventsArgs::try_parse_from([
            "events",
            "--from-block",
            "1",
            "--to-block",
            "2",
            "--query-size",
            "1",
        ])
        .unwrap();

        let (filter, query_size) = query.resolve(&provider).await.unwrap();
        let logs =
            Cast::new(&provider).get_logs_chunked(&filter, query_size.unwrap()).await.unwrap();
        assert_eq!(logs.iter().map(|log| log.block_number).collect::<Vec<_>>(), [Some(1), Some(2)]);
    }

    #[tokio::test]
    async fn decodes_known_event_and_preserves_metadata() {
        let event = Event::parse(
            "event WidgetMoved(address indexed from, address indexed to, uint256 value)",
        )
        .unwrap();
        let from = Address::repeat_byte(0x11);
        let to = Address::repeat_byte(0x22);
        let data = LogData::new_unchecked(
            vec![event.selector(), from.into_word(), to.into_word()],
            (U256::from(42),).abi_encode().into(),
        );
        let log = Log {
            inner: alloy_primitives::Log { address: Address::repeat_byte(0xaa), data },
            block_hash: Some(B256::repeat_byte(0x33)),
            block_number: Some(7),
            block_timestamp: Some(123),
            transaction_hash: Some(TxHash::repeat_byte(0xbb)),
            transaction_index: Some(2),
            log_index: Some(3),
            removed: true,
        };
        let signature = event.full_signature();
        let abi = JsonAbi::parse([signature.as_str()]).unwrap();
        let decoder = CallTraceDecoderBuilder::new().with_address_abi(log.address(), &abi).build();
        let decoded = decoder.decode_event_with_address(log.address(), log.data()).await;
        let output = EventOutput::new(log, decoded.name, decoded.params);

        assert_eq!(output.event.as_deref(), Some("WidgetMoved"));
        assert_eq!(output.params.as_ref().unwrap().len(), 3);
        assert_eq!(output.block_hash, Some(B256::repeat_byte(0x33)));
        assert_eq!(output.block_number, Some(7));
        assert_eq!(output.block_timestamp, Some(123));
        assert_eq!(output.transaction_hash, Some(TxHash::repeat_byte(0xbb)));
        assert_eq!(output.transaction_index, Some(2));
        assert_eq!(output.log_index, Some(3));
        assert!(output.removed);
    }

    #[tokio::test]
    async fn decodes_address_scoped_anonymous_events() {
        let address = Address::repeat_byte(0xaa);
        let unindexed = Event::parse("event AnonymousValue(uint256 value) anonymous").unwrap();
        let indexed =
            Event::parse("event AnonymousTransfer(address indexed from, uint256 value) anonymous")
                .unwrap();
        let abi = JsonAbi::parse([
            "event AnonymousValue(uint256 value) anonymous",
            "event AnonymousTransfer(address indexed from, uint256 value) anonymous",
        ])
        .unwrap();
        let decoder = CallTraceDecoderBuilder::new().with_address_abi(address, &abi).build();

        let decoded = decoder
            .decode_event_with_address(
                address,
                &LogData::new_unchecked(Vec::new(), (U256::from(7),).abi_encode().into()),
            )
            .await;
        assert_eq!(decoded.name.as_deref(), Some(unindexed.name.as_str()));
        assert_eq!(decoded.params.unwrap(), [("value".to_string(), "7".to_string())]);

        let from = Address::repeat_byte(0x11);
        let decoded = decoder
            .decode_event_with_address(
                address,
                &LogData::new_unchecked(
                    vec![from.into_word()],
                    (U256::from(42),).abi_encode().into(),
                ),
            )
            .await;
        assert_eq!(decoded.name.as_deref(), Some(indexed.name.as_str()));
        assert_eq!(decoded.params.as_ref().unwrap()[0], ("from".to_string(), from.to_string()));
        assert_eq!(decoded.params.as_ref().unwrap()[1], ("value".to_string(), "42".to_string()));
    }

    #[tokio::test]
    async fn decodes_proxy_and_implementation_events_at_proxy_address() {
        let address = Address::repeat_byte(0xaa);
        let proxy_abi = JsonAbi::parse(["event Upgraded(address indexed implementation)"]).unwrap();
        let implementation_abi = JsonAbi::parse(["event ValueChanged(uint256 value)"]).unwrap();
        let decoder = CallTraceDecoderBuilder::new()
            .with_address_abi(address, &implementation_abi)
            .with_address_abi(address, &proxy_abi)
            .build();

        let upgraded = proxy_abi.events().next().unwrap();
        let implementation = Address::repeat_byte(0x22);
        let decoded = decoder
            .decode_event_with_address(
                address,
                &LogData::new_unchecked(
                    vec![upgraded.selector(), implementation.into_word()],
                    Bytes::new(),
                ),
            )
            .await;
        assert_eq!(decoded.name.as_deref(), Some("Upgraded"));
        assert_eq!(decoded.params.unwrap()[0].1, implementation.to_string());

        let changed = implementation_abi.events().next().unwrap();
        let decoded = decoder
            .decode_event_with_address(
                address,
                &LogData::new_unchecked(
                    vec![changed.selector()],
                    (U256::from(9),).abi_encode().into(),
                ),
            )
            .await;
        assert_eq!(decoded.name.as_deref(), Some("ValueChanged"));
        assert_eq!(decoded.params.unwrap()[0].1, "9");
    }

    #[tokio::test]
    async fn unknown_event_falls_back_to_raw_log() {
        let topic = B256::repeat_byte(0x11);
        let log = Log {
            inner: alloy_primitives::Log {
                address: Address::repeat_byte(0xaa),
                data: LogData::new_unchecked(vec![topic], Bytes::from_static(&[1, 2, 3])),
            },
            ..Default::default()
        };
        let decoded = CallTraceDecoderBuilder::new()
            .build()
            .decode_event_with_address(log.address(), log.data())
            .await;
        let output = EventOutput::new(log, decoded.name, decoded.params);

        assert!(output.event.is_none());
        assert_eq!(output.topics, vec![topic]);
        assert_eq!(output.data, Bytes::from_static(&[1, 2, 3]));
    }

    #[test]
    fn formats_decoded_and_raw_events() {
        let decoded = EventOutput {
            address: Address::repeat_byte(0xaa),
            block_hash: Some(B256::repeat_byte(0x33)),
            block_number: Some(7),
            block_timestamp: Some(123),
            transaction_hash: Some(TxHash::repeat_byte(0xbb)),
            transaction_index: None,
            log_index: Some(3),
            removed: false,
            event: Some("Transfer".to_string()),
            params: Some(vec![EventParam { name: "value".to_string(), value: "42".to_string() }]),
            topics: vec![],
            data: Bytes::new(),
        };
        let raw = EventOutput {
            address: Address::repeat_byte(0xbb),
            block_hash: None,
            block_number: None,
            block_timestamp: None,
            transaction_hash: None,
            transaction_index: None,
            log_index: None,
            removed: false,
            event: None,
            params: None,
            topics: vec![B256::repeat_byte(0x11)],
            data: Bytes::from_static(&[0x22]),
        };

        assert_eq!(
            format_events(&[decoded, raw]),
            concat!(
                "[block 7, tx 0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, log 3] ",
                "0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa::Transfer(value: 42)\n",
                "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB\n",
                "  topic 0: 0x1111111111111111111111111111111111111111111111111111111111111111\n",
                "  data: 0x22\n",
            )
        );
    }

    #[test]
    fn serializes_structured_json_output() {
        let event = EventOutput {
            address: Address::repeat_byte(0xaa),
            block_hash: Some(B256::repeat_byte(0x33)),
            block_number: Some(7),
            block_timestamp: Some(123),
            transaction_hash: Some(TxHash::repeat_byte(0xbb)),
            transaction_index: Some(2),
            log_index: Some(3),
            removed: false,
            event: Some("Transfer".to_string()),
            params: Some(vec![EventParam { name: "value".to_string(), value: "42".to_string() }]),
            topics: vec![B256::repeat_byte(0x11)],
            data: Bytes::from_static(&[0x22]),
        };

        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["blockHash"], B256::repeat_byte(0x33).to_string());
        assert_eq!(value["blockNumber"], 7);
        assert_eq!(value["blockTimestamp"], 123);
        assert_eq!(value["event"], "Transfer");
        assert_eq!(value["params"][0]["name"], "value");
        assert_eq!(value["topics"][0], B256::repeat_byte(0x11).to_string());
        assert_eq!(value["data"], "0x22");
    }
}
