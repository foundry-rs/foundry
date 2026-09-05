use crate::{
    Cast, MAX_CONCURRENT_RPC_REQUESTS, encode_event_topic, is_range_limit_error, pretty_log,
};
use alloy_consensus::BlockHeader;
use alloy_dyn_abi::Specifier;
use alloy_ens::NameOrAddress;
use alloy_json_abi::Event;
use alloy_network::{AnyNetwork, BlockResponse, Network};
use alloy_primitives::{Address, B256, TxHash};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag, Filter, FilterBlockOption, Log, Topic};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::RpcOpts,
    utils::{self, LoadConfig},
};
use foundry_common::{fmt::UIfmt, shell};
use futures::{FutureExt, StreamExt, TryStreamExt, future::Either};
use std::{io, str::FromStr};
use tokio::signal::ctrl_c;

/// CLI arguments for `cast logs`.
#[derive(Debug, Parser)]
pub struct LogsArgs {
    #[command(flatten)]
    query: LogQueryArgs,

    /// If the RPC type and endpoints supports `eth_subscribe` stream logs instead of printing and
    /// exiting. Will continue until interrupted or TO_BLOCK is reached.
    #[arg(long)]
    subscribe: bool,

    #[command(flatten)]
    rpc: RpcOpts,
}

/// Arguments shared by commands that query logs with `eth_getLogs`.
#[derive(Debug, Parser)]
pub struct LogQueryArgs {
    /// The block height to start query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long)]
    from_block: Option<BlockId>,

    /// The block height to stop query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long)]
    to_block: Option<BlockId>,

    /// The contract address to filter on.
    #[arg(long, value_parser = NameOrAddress::from_str)]
    address: Option<Vec<NameOrAddress>>,

    /// The signature of the event to filter logs by which will be converted to the first topic or
    /// a topic to filter on.
    #[arg(value_name = "SIG_OR_TOPIC")]
    sig_or_topic: Option<String>,

    /// If used with a signature, the indexed fields of the event to filter by. Otherwise, the
    /// remaining topics of the filter.
    #[arg(value_name = "TOPICS_OR_ARGS")]
    topics_or_args: Vec<String>,

    /// Split the query into chunks of this many blocks to work around provider range/result
    /// limits.
    ///
    /// When omitted, the range is queried in a single request. Pass a value (e.g. `10000`) to
    /// fetch the logs in `query-size`-block chunks instead.
    #[arg(long, value_name = "BLOCKS")]
    query_size: Option<u64>,
}

impl LogsArgs {
    pub async fn run(self) -> Result<()> {
        let Self { query, subscribe, rpc } = self;

        let config = rpc.load_config()?;
        let provider = utils::get_provider(&config)?;
        let (filter, query_size) = query.resolve(&provider).await?;
        let cast = Cast::new(&provider);

        if !subscribe {
            let logs = match query_size {
                Some(chunk_size) => {
                    format_logs(get_logs_chunked(&provider, &filter, chunk_size).await?)?
                }
                None => format_logs(provider.get_logs(&filter).await?)?,
            };
            sh_println!("{logs}")?;
            return Ok(());
        }

        // JSON envelope intentionally unsupported for streaming: --subscribe emits NDJSON events
        // continuously; a terminal JsonEnvelope is pointless.
        // FIXME: this is a hotfix for <https://github.com/foundry-rs/foundry/issues/7682>
        //  currently the alloy `eth_subscribe` impl does not work with all transports, so we use
        // the builtin transport here for now
        let url = config.get_rpc_url_or_localhost_http()?;
        let provider = alloy_provider::ProviderBuilder::<_, _, AnyNetwork>::default()
            .connect(url.as_ref())
            .await?;
        Cast::new(&provider).subscribe(filter, &mut std::io::stdout()).await
    }
}

impl LogQueryArgs {
    /// Takes a lone positional transaction hash, if present.
    pub(super) fn take_transaction_hash(&mut self) -> Option<TxHash> {
        if self.from_block.is_none()
            && self.to_block.is_none()
            && self.address.is_none()
            && self.topics_or_args.is_empty()
            && self.query_size.is_none()
            && let Some(tx_hash) = self.sig_or_topic.as_deref().and_then(|value| value.parse().ok())
        {
            self.sig_or_topic = None;
            return Some(tx_hash);
        }
        None
    }

    /// Resolves names and block tags and builds the RPC filter.
    pub async fn resolve<P: Provider<N>, N: Network>(
        self,
        provider: &P,
    ) -> Result<(Filter, Option<u64>)> {
        let Self { from_block, to_block, address, sig_or_topic, topics_or_args, query_size } = self;

        let cast = Cast::new(&provider);
        let addresses = match address {
            Some(addresses) => Some(
                futures::future::try_join_all(
                    addresses.iter().map(|address| address.resolve(provider)),
                )
                .await?,
            ),
            None => None,
        };

        let from_block =
            convert_block_number(&provider, Some(from_block.unwrap_or_else(BlockId::earliest)))
                .await?;
        let to_block =
            convert_block_number(&provider, Some(to_block.unwrap_or_else(BlockId::latest))).await?;
        let filter = build_filter(from_block, to_block, addresses, sig_or_topic, topics_or_args)?;

        Ok((filter, query_size))
    }
}

/// Builds a Filter by first trying to parse the `sig_or_topic` as an event signature. If
/// successful, `topics_or_args` is parsed as indexed inputs and converted to topics. Otherwise,
/// `sig_or_topic` is prepended to `topics_or_args` and used as raw topics.
fn build_filter(
    from_block: Option<BlockNumberOrTag>,
    to_block: Option<BlockNumberOrTag>,
    address: Option<Vec<Address>>,
    sig_or_topic: Option<String>,
    topics_or_args: Vec<String>,
) -> Result<Filter> {
    let topics = match sig_or_topic {
        Some(sig_or_topic) => match foundry_common::abi::get_event(&sig_or_topic) {
            Ok(event) => event_topics(&event, &topics_or_args)?,
            Err(_) => raw_topics([vec![sig_or_topic], topics_or_args].concat())?,
        },
        None => Default::default(),
    };

    let mut filter = Filter {
        block_option: FilterBlockOption::Range { from_block, to_block },
        topics,
        ..Default::default()
    };
    if let Some(address) = address {
        filter = filter.address(address);
    }
    Ok(filter)
}

/// Encodes `args` as the indexed topics of `event`; empty arguments match any value.
fn event_topics(event: &Event, args: &[String]) -> Result<[Topic; 4]> {
    let mut topics = vec![Topic::from(event.selector())];
    for (input, arg) in event.inputs.iter().filter(|input| input.indexed).zip(args) {
        let kind = input.resolve()?;
        topics.push(if arg.is_empty() {
            Topic::default()
        } else {
            Topic::from(encode_event_topic(&kind.coerce_str(arg)?))
        });
    }
    topics.resize(4, Topic::default());
    Ok(topics.try_into().unwrap())
}

/// Parses raw topic hashes; empty topics match any value.
fn raw_topics(topics: Vec<String>) -> Result<[Topic; 4]> {
    let mut topics = topics
        .into_iter()
        .map(|topic| {
            Ok(if topic.is_empty() {
                Topic::default()
            } else {
                Topic::from(B256::from_str(&topic)?)
            })
        })
        .collect::<Result<Vec<_>>>()?;
    topics.resize(4, Topic::default());
    Ok(topics.try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::keccak256;

    const ADDRESS: &str = "0x4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38";
    const TRANSFER_SIG: &str = "Transfer(address indexed,address indexed,uint256)";
    const TRANSFER_TOPIC: &str =
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

    fn filter(sig_or_topic: &str, args: &[&str]) -> Result<Filter> {
        build_filter(
            None,
            None,
            None,
            Some(sig_or_topic.to_string()),
            args.iter().map(|s| s.to_string()).collect(),
        )
    }

    fn topics(topics: [Topic; 4]) -> Filter {
        Filter { topics, ..Default::default() }
    }

    #[test]
    fn builds_filters() {
        let transfer_topic = B256::from_str(TRANSFER_TOPIC).unwrap();
        let addr: Address = ADDRESS.parse().unwrap();
        let addr_topic = Topic::from(B256::left_padding_from(addr.as_slice()));
        let any = Topic::default;

        let from_block = Some(BlockNumberOrTag::from(1337));
        let to_block = Some(BlockNumberOrTag::Latest);
        let basic = build_filter(from_block, to_block, Some(vec![addr]), None, vec![]).unwrap();
        assert_eq!(
            basic,
            Filter {
                block_option: FilterBlockOption::Range { from_block, to_block },
                address: addr.into(),
                topics: Default::default(),
            }
        );

        let cases: [(&str, &[&str], [Topic; 4]); 8] = [
            (TRANSFER_SIG, &[], [transfer_topic.into(), any(), any(), any()]),
            (TRANSFER_SIG, &[ADDRESS], [transfer_topic.into(), addr_topic.clone(), any(), any()]),
            (TRANSFER_SIG, &["", ADDRESS], [transfer_topic.into(), any(), addr_topic, any()]),
            (
                TRANSFER_TOPIC,
                &[TRANSFER_TOPIC],
                [transfer_topic.into(), transfer_topic.into(), any(), any()],
            ),
            (
                TRANSFER_TOPIC,
                &["", TRANSFER_TOPIC],
                [transfer_topic.into(), any(), transfer_topic.into(), any()],
            ),
            (
                "event Owned(uint256 value, address indexed owner)",
                &[ADDRESS],
                [
                    Event::parse("event Owned(uint256 value, address indexed owner)")
                        .unwrap()
                        .selector()
                        .into(),
                    B256::left_padding_from(addr.as_slice()).into(),
                    any(),
                    any(),
                ],
            ),
            (
                "event Message(string indexed value)",
                &["hello"],
                [
                    Event::parse("event Message(string indexed value)").unwrap().selector().into(),
                    keccak256("hello").into(),
                    any(),
                    any(),
                ],
            ),
            (
                "Swap(address indexed from, address indexed to, uint256 value)",
                &[],
                [
                    Event::parse(
                        "event Swap(address indexed from, address indexed to, uint256 value)",
                    )
                    .unwrap()
                    .selector()
                    .into(),
                    any(),
                    any(),
                    any(),
                ],
            ),
        ];
        for (sig_or_topic, args, expected) in cases {
            assert_eq!(filter(sig_or_topic, args).unwrap(), topics(expected), "{sig_or_topic}");
        }

        let multiple = build_filter(
            None,
            None,
            Some(vec![Address::ZERO, addr]),
            Some(TRANSFER_TOPIC.to_string()),
            vec![],
        )
        .unwrap();
        assert_eq!(
            multiple,
            Filter {
                address: vec![Address::ZERO, addr].into(),
                topics: [transfer_topic.into(), any(), any(), any()],
                ..Default::default()
            }
        );
    }

    #[test]
    fn rejects_invalid_arguments_and_topics() {
        let cases = [
            (TRANSFER_SIG, &["1234"][..], "parser error:\n1234\n^\ninvalid string length"),
            ("asdasdasd", &[], "odd number of digits"),
            (ADDRESS, &[], "invalid string length"),
            (TRANSFER_TOPIC, &["1234"], "invalid string length"),
        ];
        for (sig_or_topic, args, expected) in cases {
            let err = filter(sig_or_topic, args).unwrap_err().to_string().to_lowercase();
            assert_eq!(err, expected, "{sig_or_topic}");
        }
    }
}

pub(crate) fn get_logs_bisecting<'a, P: Provider<N>, N: Network>(
    provider: &'a P,
    filter: &'a Filter,
    from: u64,
    to: u64,
) -> futures::future::BoxFuture<'a, Result<Vec<Log>>>
where
    P: Clone + Unpin,
{
    Box::pin(async move {
        let range_filter = filter.clone().from_block(from).to_block(to);
        match provider.get_logs(&range_filter).await {
            Ok(logs) => Ok(logs),
            Err(e) => {
                // Only bisect range-limit errors with room left to split; surface anything
                // else immediately.
                if from >= to || !is_range_limit_error(&e) {
                    return Err(e.into());
                }

                // Bisect sequentially: this path is only reached after a provider failure, so
                // fanning out concurrently here would risk amplifying rate-limit errors and
                // would defeat the top-level concurrency cap.
                let mid = from + (to - from) / 2;
                let mut left = get_logs_bisecting(provider, filter, from, mid).await?;
                let right = get_logs_bisecting(provider, filter, mid + 1, to).await?;
                left.extend(right);
                Ok(left)
            }
        }
    })
}

pub(crate) async fn get_logs_chunked_concurrent<P: Provider<N> + Clone + Unpin, N: Network>(
    provider: &P,
    filter: &Filter,
    from: u64,
    to: u64,
    chunk_size: u64,
) -> Result<Vec<Log>>
where
    P: Clone + Unpin,
{
    let chunk_ranges = (from..=to)
        .step_by(chunk_size as usize)
        .map(|start| (start, start.saturating_add(chunk_size - 1).min(to)));

    // `buffered` preserves input order, so results stay ordered by block. `try_collect` stops
    // early and surfaces the error if any chunk ultimately fails.
    let chunks: Vec<Vec<Log>> =
        futures::stream::iter(chunk_ranges)
            .map(|(start, end)| {
                let filter = filter.clone();
                let provider = provider.clone();
                async move {
                    crate::cmd::logs::get_logs_bisecting(&provider, &filter, start, end).await
                }
            })
            .buffered(MAX_CONCURRENT_RPC_REQUESTS)
            .try_collect()
            .await?;

    Ok(chunks.into_iter().flatten().collect())
}

pub(crate) async fn resolve_block_tag<P: Provider<N> + Clone + Unpin, N: Network>(
    provider: &P,
    tag: BlockNumberOrTag,
) -> Result<u64> {
    match tag {
        BlockNumberOrTag::Number(number) => Ok(number),
        BlockNumberOrTag::Earliest => Ok(0),
        tag => {
            let block = provider
                .get_block(BlockId::Number(tag))
                .await?
                .ok_or_else(|| eyre::eyre!("could not resolve block tag `{tag}`"))?;
            Ok(block.header().number())
        }
    }
}

pub(crate) async fn resolve_block_range<P: Provider<N> + Clone + Unpin, N: Network>(
    provider: &P,
    filter: &Filter,
) -> Result<Option<(u64, u64)>> {
    let FilterBlockOption::Range { from_block, to_block } = &filter.block_option else {
        return Ok(None);
    };

    let from_tag = from_block.unwrap_or(BlockNumberOrTag::Earliest);
    let to_tag = to_block.unwrap_or(BlockNumberOrTag::Latest);

    // `pending` is not a concrete canonical range boundary; don't chunk it, so the single
    // request preserves the provider's native `pending` semantics.
    if from_tag.is_pending() || to_tag.is_pending() {
        return Ok(None);
    }

    let from = crate::cmd::logs::resolve_block_tag(provider, from_tag).await?;
    // Resolve identical tags only once so a moving head (e.g. `latest`..`latest`) can't yield
    // an inconsistent range.
    let to = if from_tag == to_tag {
        from
    } else {
        crate::cmd::logs::resolve_block_tag(provider, to_tag).await?
    };
    Ok(Some((from, to)))
}

pub(crate) async fn get_logs_chunked<P: Provider<N> + Clone + Unpin, N: Network>(
    provider: &P,
    filter: &Filter,
    chunk_size: u64,
) -> Result<Vec<Log>>
where
    P: Clone + Unpin,
{
    // Only chunk a finite block-number range larger than one chunk; `chunk_size == 0`
    // disables chunking and falls back to a single request.
    let Some((from, to)) = resolve_block_range(provider, filter).await? else {
        return provider.get_logs(filter).await.map_err(Into::into);
    };
    // Inverted range yields no logs; warn instead of returning empty silently.
    if from > to {
        sh_warn!(
            "requested block range is inverted (from-block {from} > to-block {to}); no logs to return"
        )?;
        return Ok(vec![]);
    }
    if chunk_size == 0 || to - from < chunk_size {
        return provider.get_logs(filter).await.map_err(Into::into);
    }

    get_logs_chunked_concurrent(provider, filter, from, to, chunk_size).await
}

pub(crate) async fn convert_block_number<P: Provider<N> + Clone + Unpin, N: Network>(
    provider: &P,
    block: Option<BlockId>,
) -> Result<Option<BlockNumberOrTag>> {
    match block {
        Some(BlockId::Number(number)) => Ok(Some(number)),
        Some(BlockId::Hash(hash)) => {
            let block = provider.get_block_by_hash(hash.block_hash).await?;
            Ok(block.map(|block| block.header().number().into()))
        }
        None => Ok(None),
    }
}

pub(crate) fn format_logs(logs: Vec<Log>) -> Result<String> {
    if shell::is_json() {
        Ok(serde_json::to_string(&logs)?)
    } else {
        Ok(logs.iter().map(pretty_log).collect::<Vec<_>>().join("\n"))
    }
}
