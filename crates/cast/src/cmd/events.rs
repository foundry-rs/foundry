use super::logs::LogQueryArgs;
use crate::{
    Cast, MAX_CONCURRENT_RPC_REQUESTS,
    traces::{
        CallTraceDecoderBuilder, DecodedEvent,
        identifier::{ExternalIdentifier, SignaturesIdentifier},
    },
};
use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::JsonAbi;
use alloy_network::Network;
use alloy_primitives::{Address, B256, Bytes, TxHash};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, Log};
use clap::{ArgGroup, Parser};
use eyre::Result;
use foundry_cli::{
    json::print_json_object,
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, load_config_from_provider},
};
use foundry_common::{
    ContractsByArtifact, compile::ProjectCompiler, fmt::serialize_value_as_json, shell,
};
use foundry_config::{Chain, Config};
use futures::StreamExt;
use serde::{Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
};

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

    /// Use current project artifacts for event decoding.
    ///
    /// Only supported when querying by transaction hash.
    #[arg(long, visible_alias = "la")]
    with_local_artifacts: bool,
}

impl EventsArgs {
    pub async fn run(self) -> Result<()> {
        let figment =
            self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self.etherscan);
        let mut config = load_config_from_provider(figment)?;
        let Self { tx_hash, mut query, etherscan: _, rpc: _, with_local_artifacts } = self;
        let tx_hash = tx_hash.or_else(|| query.take_transaction_hash());
        if with_local_artifacts && tx_hash.is_none() {
            eyre::bail!("--with-local-artifacts is only supported with a transaction hash");
        }
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

        let local_abis = if with_local_artifacts {
            local_event_abis(&provider, &logs, &config).await?
        } else {
            BTreeMap::new()
        };
        let events = decode_logs(logs, &config, explorer_chain, local_abis).await?;
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

async fn local_event_abis<N: Network, P: Provider<N>>(
    provider: &P,
    logs: &[Log],
    config: &Config,
) -> Result<BTreeMap<Address, JsonAbi>> {
    let addresses = logs.iter().map(Log::address).collect::<BTreeSet<_>>();
    let Some(block_number) = logs.first().and_then(|log| log.block_number) else {
        return Ok(BTreeMap::new());
    };

    let mut bytecodes = BTreeMap::new();
    let mut requests = futures::stream::iter(addresses.iter().copied())
        .map(|address| async move {
            (address, provider.get_code_at(address).block_id(BlockId::number(block_number)).await)
        })
        .buffer_unordered(MAX_CONCURRENT_RPC_REQUESTS);
    while let Some((address, code)) = requests.next().await {
        match code {
            Ok(code) if !code.is_empty() => {
                bytecodes.insert(address, code);
            }
            Ok(_) => {}
            Err(err) => sh_warn!("Failed to fetch code for {address}: {err}")?,
        }
    }

    let project = config.project()?;
    let output = ProjectCompiler::new().quiet(true).compile(&project)?;
    let contracts = ContractsByArtifact::from(output);
    Ok(addresses
        .iter()
        .filter_map(|&address| {
            let code = bytecodes.get(&address)?;
            let (_, contract) = contracts.find_by_deployed_code_exact_unique(code)?;
            Some((address, contract.abi.clone()))
        })
        .collect())
}

async fn decode_logs(
    logs: Vec<Log>,
    config: &Config,
    explorer_chain: Chain,
    local_abis: BTreeMap<Address, JsonAbi>,
) -> Result<Vec<EventOutput>> {
    let signature_identifier = SignaturesIdentifier::from_config(config)?;
    let mut builder = CallTraceDecoderBuilder::new()
        .with_signature_identifier(signature_identifier)
        .with_networks(config.networks)
        .with_chain_id(config.chain.map(|chain| chain.id()));

    let mut externally_resolved = BTreeSet::new();
    if let Some(mut identifier) = ExternalIdentifier::new(config, Some(explorer_chain))? {
        let addresses =
            logs.iter().map(Log::address).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        for (address, result) in identifier.get_abis(&addresses).await {
            match result {
                Ok((abis, complete)) => {
                    externally_resolved.insert(address);
                    if !complete {
                        sh_warn!("Only partially resolved proxy ABI chain for {address}")?;
                    }
                    for abi in abis {
                        builder = builder.with_address_events(address, &abi);
                    }
                }
                Err(err) => sh_warn!("Failed to fetch ABI for {address}: {err}")?,
            }
        }
    }

    for (address, abi) in local_abis {
        if !externally_resolved.contains(&address) {
            builder = builder.with_address_events(address, &abi);
        }
    }

    let decoder = builder.build();
    let events = futures::stream::iter(logs)
        .map(|log| async {
            let decoded =
                decoder.decode_event_with_address_signature(log.address(), log.data()).await;
            EventOutput::new(log, decoded)
        })
        .buffered(MAX_CONCURRENT_RPC_REQUESTS)
        .collect()
        .await;
    Ok(events)
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
    fn new(log: Log, decoded: DecodedEvent) -> Self {
        let params = decoded.params.map(|params| {
            params
                .into_iter()
                .enumerate()
                .map(|(index, (name, display_value, value))| EventParam {
                    name: if name.is_empty() { format!("param{index}") } else { name },
                    value,
                    display_value,
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
            event: decoded.name,
            params,
            topics: log.topics().to_vec(),
            data: log.data().data.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct EventParam {
    name: String,
    #[serde(serialize_with = "serialize_abi_value")]
    value: DynSolValue,
    #[serde(skip)]
    display_value: String,
}

fn serialize_abi_value<S>(value: &DynSolValue, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serialize_value_as_json(value.clone(), None, true)
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

/// Formats decoded and raw events for human-readable output.
///
/// # Example
///
/// ```text
/// [block 1, tx 0xabc..., log 0] 0x123...::Transfer(address,uint256) { from: 0x456..., value: 1 }
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
            let _ = write!(output, "{}::{name}", event.address);
            if let Some(params) = &event.params {
                output.push_str(" { ");
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        output.push_str(", ");
                    }
                    let _ = write!(output, "{}: {}", param.name, param.display_value);
                }
                output.push_str(" }");
            }
            output.push('\n');
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
    use alloy_primitives::{Function, U256};

    #[test]
    fn validates_event_sources() {
        assert!(EventsArgs::try_parse_from(["events"]).is_err());
        assert!(
            EventsArgs::try_parse_from(["events", "--tx-hash", &TxHash::ZERO.to_string()]).is_ok()
        );
        let EventsArgs { tx_hash, mut query, .. } =
            EventsArgs::try_parse_from(["events", &TxHash::ZERO.to_string()]).unwrap();
        assert_eq!(tx_hash.or_else(|| query.take_transaction_hash()), Some(TxHash::ZERO));
        let EventsArgs { mut query, .. } = EventsArgs::try_parse_from([
            "events",
            &TxHash::ZERO.to_string(),
            "--address",
            &Address::ZERO.to_string(),
        ])
        .unwrap();
        assert!(query.take_transaction_hash().is_none());
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

    #[test]
    fn serializes_abi_values_instead_of_display_values() {
        let output = EventOutput::new(
            Log::default(),
            DecodedEvent {
                name: Some("Value(uint256)".to_string()),
                params: Some(vec![(
                    "value".to_string(),
                    "19705728070 [1.97e10]".to_string(),
                    DynSolValue::Uint(U256::from(19_705_728_070u64), 256),
                )]),
            },
        );

        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["params"][0]["value"], "19705728070");
        assert!(format_events(&[output]).contains("19705728070 [1.97e10]"));
    }

    #[test]
    fn formats_function_values_without_json_serialization() {
        let output = EventOutput::new(
            Log::default(),
            DecodedEvent {
                name: Some("Callback(function)".to_string()),
                params: Some(vec![(
                    "callback".to_string(),
                    "0x111111111111111111111111111111111111111111111111".to_string(),
                    DynSolValue::Function(Function::new([0x11; 24])),
                )]),
            },
        );

        assert!(format_events(&[output]).contains(
            "Callback(function) { callback: 0x111111111111111111111111111111111111111111111111 }"
        ));
    }

    #[test]
    fn formats_decoded_and_raw_events() {
        let decoded = EventOutput {
            address: Address::repeat_byte(0xaa),
            block_hash: Some(B256::repeat_byte(0x33)),
            block_number: Some(7),
            block_timestamp: Some(123),
            transaction_hash: Some(TxHash::repeat_byte(0xbb)),
            transaction_index: Some(2),
            log_index: Some(3),
            removed: false,
            event: Some("Transfer(address,address,uint256)".to_string()),
            params: Some(vec![EventParam {
                name: "value".to_string(),
                value: DynSolValue::Uint(U256::from(42), 256),
                display_value: "42".to_string(),
            }]),
            topics: vec![B256::repeat_byte(0x11)],
            data: Bytes::from_static(&[0x22]),
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

        let value = serde_json::to_value(&decoded).unwrap();
        assert_eq!(value["blockNumber"], 7);
        assert_eq!(value["event"], "Transfer(address,address,uint256)");
        assert_eq!(value["params"][0]["name"], "value");
        assert_eq!(value["params"][0]["value"], "42");
        assert_eq!(value["data"], "0x22");

        assert_eq!(
            format_events(&[decoded, raw]),
            concat!(
                "[block 7, tx 0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, log 3] ",
                "0xaAaAaAaaAaAaAaaAaAAAAAAAAaaaAaAaAaaAaaAa::Transfer(address,address,uint256) { value: 42 }\n",
                "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB\n",
                "  topic 0: 0x1111111111111111111111111111111111111111111111111111111111111111\n",
                "  data: 0x22\n",
            )
        );
    }
}
