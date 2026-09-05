use super::MAX_CONCURRENT_RPC_REQUESTS;
use crate::args::encode_event_topic;
use alloy_consensus::BlockHeader;
use alloy_dyn_abi::Specifier;
use alloy_ens::NameOrAddress;
use alloy_json_abi::Event;
use alloy_json_rpc::RpcError;
use alloy_network::{AnyNetwork, BlockResponse, Network};
use alloy_primitives::{Address, B256, TxHash};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag, Filter, FilterBlockOption, Log, Topic};
use alloy_transport::TransportErrorKind;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::RpcOpts,
    utils::{self, LoadConfig},
};
use foundry_common::{fmt::UIfmt, shell};
use futures::{FutureExt, StreamExt, TryStreamExt, future::Either};
use std::{io::Write, str::FromStr};
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
        let output = &mut std::io::stdout();
        let mut subscription = provider.subscribe_logs(&filter).await?.into_stream();

        // Subscribe to blocks when a `to_block` is set so the stream ends once it is passed.
        let to_block_number = filter.get_to_block();
        let mut block_subscription = match to_block_number {
            Some(_) => Some(provider.subscribe_blocks().await?.into_stream()),
            None => None,
        };

        let format_json = shell::is_json();
        if format_json {
            write!(output, "[")?;
        }

        let mut first = true;
        loop {
            tokio::select! {
                block = match &mut block_subscription {
                    Some(bs) => Either::Left(bs.next().fuse()),
                    None => Either::Right(futures::future::pending()),
                } => {
                    if let (Some(block), Some(to_block)) = (block, to_block_number)
                        && block.number() > to_block
                    {
                        break;
                    }
                },
                log = subscription.next() => {
                    if format_json {
                        if !first {
                            write!(output, ",")?;
                        }
                        first = false;
                        write!(output, "{}", serde_json::to_string(&log).unwrap())?;
                    } else {
                        writeln!(output, "{}", pretty_log(&log))?;
                    }
                },
                // Break on the cancel signal so the JSON array is still closed.
                _ = ctrl_c() => break,
                else => break,
            }
        }

        if format_json {
            write!(output, "]")?;
        }
        Ok(())
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

/// Fetches logs for the inclusive `[from, to]` range, recursively bisecting on failure.
fn get_logs_bisecting<'a, P: Provider<N>, N: Network>(
    provider: &'a P,
    filter: &'a Filter,
    from: u64,
    to: u64,
) -> futures::future::BoxFuture<'a, Result<Vec<Log>>> {
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

/// Retrieves logs for the inclusive `[from, to]` range using concurrent chunked requests.
async fn get_logs_chunked_concurrent<P: Provider<N>, N: Network>(
    provider: &P,
    filter: &Filter,
    from: u64,
    to: u64,
    chunk_size: u64,
) -> Result<Vec<Log>> {
    let chunk_ranges = (from..=to)
        .step_by(chunk_size as usize)
        .map(|start| (start, start.saturating_add(chunk_size - 1).min(to)));

    // `buffered` preserves input order, so results stay ordered by block. `try_collect` stops
    // early and surfaces the error if any chunk ultimately fails.
    let chunks: Vec<Vec<Log>> = futures::stream::iter(chunk_ranges)
        .map(|(start, end)| get_logs_bisecting(provider, filter, start, end))
        .buffered(MAX_CONCURRENT_RPC_REQUESTS)
        .try_collect()
        .await?;

    Ok(chunks.into_iter().flatten().collect())
}

/// Resolves a [`BlockNumberOrTag`] to a concrete block number, querying the provider for tags.
async fn resolve_block_tag<P: Provider<N>, N: Network>(
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

/// Resolves the filter's block range to concrete block numbers.
///
/// Returns `None` when the filter does not target a block-number range (e.g. it filters by
/// block hash), in which case chunking is not possible. Tags such as `latest` and `earliest`
/// are resolved against the provider so that the common case (`--to-block` defaulting to
/// `latest`) can still be chunked.
async fn resolve_block_range<P: Provider<N>, N: Network>(
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

    let from = resolve_block_tag(provider, from_tag).await?;
    // Resolve identical tags only once so a moving head (e.g. `latest`..`latest`) can't yield
    // an inconsistent range.
    let to = if from_tag == to_tag { from } else { resolve_block_tag(provider, to_tag).await? };
    Ok(Some((from, to)))
}

/// Retrieves logs, splitting the request into fixed-size block chunks when needed.
pub(super) async fn get_logs_chunked<P: Provider<N>, N: Network>(
    provider: &P,
    filter: &Filter,
    chunk_size: u64,
) -> Result<Vec<Log>> {
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

async fn convert_block_number<P: Provider<N>, N: Network>(
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

fn format_logs(logs: Vec<Log>) -> Result<String> {
    if shell::is_json() {
        Ok(serde_json::to_string(&logs)?)
    } else {
        Ok(logs.iter().map(pretty_log).collect::<Vec<_>>().join("\n"))
    }
}

/// Renders a log as an indented list item.
fn pretty_log(log: &impl UIfmt) -> String {
    log.pretty()
        .replacen('\n', "- ", 1) // Remove empty first line
        .replace('\n', "\n  ") // Indent
}

/// Returns `true` if `err` is a provider range/result-size limit that retrying over a smaller
/// range can fix. Network, auth, rate-limit, and malformed-response errors return `false`.
fn is_range_limit_error(err: &RpcError<TransportErrorKind>) -> bool {
    // Only HTTP 413 (payload too large) is fixable by a smaller range; other transport errors
    // (network, auth 401/403, rate-limit 429) are not.
    if let RpcError::Transport(kind) = err {
        return kind.as_http_error().is_some_and(|http| http.status == 413);
    }

    // Range/result-size limits are reported as JSON-RPC server error responses; every other
    // variant falls through to `false`.
    let RpcError::ErrorResp(payload) = err else { return false };
    let message = payload.message.to_ascii_lowercase();

    // Phrases providers use for range/result-size limits, kept specific so rate-limit/quota
    // wording (e.g. "no more than 10 requests per second") doesn't match.
    const RANGE_LIMIT_HINTS: &[&str] = &[
        "block range",
        "blocks range",
        "range is too",
        "range too",
        "returned more than",
        "response size",
        "result set",
        "too many results",
        "too many blocks",
        "maximum block range",
        "max block range",
    ];
    RANGE_LIMIT_HINTS.iter().any(|hint| message.contains(hint))
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

#[cfg(test)]
mod logs_bisecting {
    use super::*;
    use alloy_json_rpc::{RequestPacket, ResponsePacket, SerializedRequest};
    use alloy_provider::ProviderBuilder;
    use alloy_rpc_client::RpcClient;
    use alloy_transport::{
        TransportError, TransportFut,
        mock::{Asserter, MockTransport},
    };
    use std::{
        sync::{Arc, Mutex},
        task::{Context, Poll},
    };
    use tower::Service;

    fn log_at(block: u64) -> Log {
        Log { block_number: Some(block), ..Default::default() }
    }

    /// Mock transport that records the `eth_getLogs` `[fromBlock, toBlock]` ranges it is asked for
    /// while delegating the actual responses to a FIFO [`Asserter`].
    #[derive(Clone)]
    struct RecordingTransport {
        inner: MockTransport,
        ranges: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl RecordingTransport {
        fn new(asserter: Asserter) -> Self {
            Self { inner: MockTransport::new(asserter), ranges: Arc::new(Mutex::new(Vec::new())) }
        }

        fn record(&self, req: &SerializedRequest) {
            if req.method() != "eth_getLogs" {
                return;
            }
            let Some(params) = req.params() else { return };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(params.get()) else { return };
            let Some(filter) = value.get(0) else { return };
            let field =
                |name| filter.get(name).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            self.ranges.lock().unwrap().push((field("fromBlock"), field("toBlock")));
        }
    }

    impl Service<RequestPacket> for RecordingTransport {
        type Response = ResponsePacket;
        type Error = TransportError;
        type Future = TransportFut<'static>;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.inner.poll_ready(cx)
        }

        fn call(&mut self, req: RequestPacket) -> Self::Future {
            match &req {
                RequestPacket::Single(req) => self.record(req),
                RequestPacket::Batch(reqs) => reqs.iter().for_each(|req| self.record(req)),
            }
            self.inner.call(req)
        }
    }

    // A range-limit failure splits depth-first into [0,1]/[2,3] and aggregates in range order.
    #[tokio::test]
    async fn bisects_failed_range_and_aggregates_in_order() {
        let asserter = Asserter::new();
        asserter.push_failure_msg("query returned more than 10000 results");
        asserter.push_success(&vec![log_at(0)]);
        asserter.push_success(&vec![log_at(2)]);

        let transport = RecordingTransport::new(asserter);
        let ranges = transport.ranges.clone();
        let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
            .connect_client(RpcClient::new(transport, true));

        let logs = get_logs_bisecting(&provider, &Filter::new(), 0, 3).await.unwrap();
        let blocks: Vec<_> = logs.iter().map(|l| l.block_number).collect();
        assert_eq!(blocks, vec![Some(0), Some(2)]);

        // The original range fails, then bisection requests exactly the two halves in order.
        let ranges = ranges.lock().unwrap();
        assert_eq!(
            *ranges,
            vec![
                ("0x0".to_string(), "0x3".to_string()),
                ("0x0".to_string(), "0x1".to_string()),
                ("0x2".to_string(), "0x3".to_string()),
            ]
        );
    }

    // A single-block failure can't be split, so the error is surfaced.
    #[tokio::test]
    async fn surfaces_single_block_failure() {
        let asserter = Asserter::new();
        asserter.push_failure_msg("query returned more than 10000 results");

        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter);

        let err = get_logs_bisecting(&provider, &Filter::new(), 5, 5).await.unwrap_err();
        assert!(err.to_string().contains("more than 10000 results"), "got: {err}");
    }

    // A non-range error fails after one request instead of bisecting.
    #[tokio::test]
    async fn does_not_bisect_non_range_errors() {
        let asserter = Asserter::new();
        asserter.push_failure_msg("unauthorized: invalid api key");

        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter);

        let err = get_logs_bisecting(&provider, &Filter::new(), 0, 3).await.unwrap_err();
        assert!(err.to_string().contains("unauthorized"), "got: {err}");
    }
}
