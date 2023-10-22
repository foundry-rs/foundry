use std::{io, str::FromStr};

use cast::Cast;
use clap::Parser;
use ethers::{providers::Middleware, types::NameOrAddress};
use ethers_core::{
    abi::{
        token::{LenientTokenizer, StrictTokenizer, Tokenizer},
        Address, Event, HumanReadableParser, ParamType, RawTopicFilter, Token, Topic, TopicFilter,
    },
    types::{BlockId, BlockNumber, Filter, FilterBlockOption, ValueOrArray, H256, U256},
};
use eyre::{Result, WrapErr};
use foundry_cli::{opts::EthereumOpts, utils};

use foundry_config::Config;
use itertools::Itertools;

/// CLI arguments for `cast logs`.
#[derive(Debug, Parser)]
pub struct LogsArgs {
    /// The block height to start query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[clap(long)]
    from_block: Option<BlockId>,

    /// The block height to stop query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[clap(long)]
    to_block: Option<BlockId>,

    /// The contract address to filter on.
    #[clap(
        long,
        value_parser = NameOrAddress::from_str
    )]
    address: Option<NameOrAddress>,

    /// The signature of the event to filter logs by which will be converted to the first topic or
    /// a topic to filter on.
    #[clap(value_name = "SIG_OR_TOPIC")]
    sig_or_topic: Option<String>,

    /// If used with a signature, the indexed fields of the event to filter by. Otherwise, the
    /// remaining topics of the filter.
    #[clap(value_name = "TOPICS_OR_ARGS")]
    topics_or_args: Vec<String>,

    /// If the RPC type and endpoints supports `eth_subscribe` stream logs instead of printing and
    /// exiting. Will continue until interrupted or TO_BLOCK is reached.
    #[clap(long)]
    subscribe: bool,

    /// Print the logs as JSON.s
    #[clap(long, short, help_heading = "Display options")]
    json: bool,

    #[clap(flatten)]
    eth: EthereumOpts,
}

impl LogsArgs {
    pub async fn run(self) -> Result<()> {
        let LogsArgs {
            from_block,
            to_block,
            address,
            sig_or_topic,
            topics_or_args,
            subscribe,
            json,
            eth,
        } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;

        let cast = Cast::new(&provider);

        let address = match address {
            Some(address) => {
                let address = match address {
                    NameOrAddress::Name(name) => provider.resolve_name(&name).await?,
                    NameOrAddress::Address(address) => address,
                };
                Some(address)
            }
            None => None,
        };

        let from_block = cast.convert_block_number(from_block).await?;
        let to_block = cast.convert_block_number(to_block).await?;

        let filter = build_filter(from_block, to_block, address, sig_or_topic, topics_or_args)?;

        if !subscribe {
            let logs = cast.filter_logs(filter, json).await?;

            println!("{}", logs);

            return Ok(())
        }

        let mut stdout = io::stdout();
        cast.subscribe(filter, &mut stdout, json).await?;

        Ok(())
    }
}

/// Builds a Filter by first trying to parse the `sig_or_topic` as an event signature. If
/// successful, `topics_or_args` is parsed as indexed inputs and converted to topics. Otherwise,
/// `sig_or_topic` is prepended to `topics_or_args` and used as raw topics.
fn build_filter(
    from_block: Option<BlockNumber>,
    to_block: Option<BlockNumber>,
    address: Option<Address>,
    sig_or_topic: Option<String>,
    topics_or_args: Vec<String>,
) -> Result<Filter, eyre::Error> {
    let block_option = FilterBlockOption::Range { from_block, to_block };
    let topic_filter = match sig_or_topic {
        // Try and parse the signature as an event signature
        Some(sig_or_topic) => match HumanReadableParser::parse_event(sig_or_topic.as_str()) {
            Ok(event) => build_filter_event_sig(event, topics_or_args)?,
            Err(_) => {
                let topics = [vec![sig_or_topic], topics_or_args].concat();
                build_filter_topics(topics)?
            }
        },
        None => TopicFilter::default(),
    };

    // Convert from TopicFilter to Filter
    let topics =
        vec![topic_filter.topic0, topic_filter.topic1, topic_filter.topic2, topic_filter.topic3]
            .into_iter()
            .map(|topic| match topic {
                Topic::Any => None,
                Topic::This(topic) => Some(ValueOrArray::Value(Some(topic))),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();

    let filter = Filter {
        block_option,
        address: address.map(ValueOrArray::Value),
        topics: [topics[0].clone(), topics[1].clone(), topics[2].clone(), topics[3].clone()],
    };

    Ok(filter)
}

/// Creates a TopicFilter from the given event signature and arguments.
fn build_filter_event_sig(event: Event, args: Vec<String>) -> Result<TopicFilter, eyre::Error> {
    let args = args.iter().map(|arg| arg.as_str()).collect::<Vec<_>>();

    // Match the args to indexed inputs. Enumerate so that the ordering can be restored
    // when merging the inputs with arguments and without arguments
    let (with_args, without_args): (Vec<_>, Vec<_>) = event
        .inputs
        .iter()
        .zip(args)
        .filter(|(input, _)| input.indexed)
        .map(|(input, arg)| (&input.kind, arg))
        .enumerate()
        .partition(|(_, (_, arg))| !arg.is_empty());

    // Only parse the inputs with arguments
    let indexed_tokens =
        parse_params(with_args.clone().into_iter().map(|(_, p)| p).collect::<Vec<_>>(), true)?;

    // Merge the inputs restoring the original ordering
    let mut tokens = with_args
        .into_iter()
        .zip(indexed_tokens)
        .map(|((i, _), t)| (i, Some(t)))
        .chain(without_args.into_iter().map(|(i, _)| (i, None)))
        .sorted_by(|(i1, _), (i2, _)| i1.cmp(i2))
        .map(|(_, token)| token)
        .collect::<Vec<_>>();

    tokens.resize(3, None);

    let raw = RawTopicFilter {
        topic0: tokens[0].clone().map_or(Topic::Any, Topic::This),
        topic1: tokens[1].clone().map_or(Topic::Any, Topic::This),
        topic2: tokens[2].clone().map_or(Topic::Any, Topic::This),
    };

    // Let filter do the hardwork of converting arguments to topics
    Ok(event.filter(raw)?)
}

/// Creates a TopicFilter from raw topic hashes.
fn build_filter_topics(topics: Vec<String>) -> Result<TopicFilter, eyre::Error> {
    let mut topics = topics
        .into_iter()
        .map(|topic| if topic.is_empty() { Ok(None) } else { H256::from_str(&topic).map(Some) })
        .collect::<Result<Vec<_>, _>>()?;

    topics.resize(4, None);

    Ok(TopicFilter {
        topic0: topics[0].map_or(Topic::Any, Topic::This),
        topic1: topics[1].map_or(Topic::Any, Topic::This),
        topic2: topics[2].map_or(Topic::Any, Topic::This),
        topic3: topics[3].map_or(Topic::Any, Topic::This),
    })
}

fn parse_params<'a, I: IntoIterator<Item = (&'a ParamType, &'a str)>>(
    params: I,
    lenient: bool,
) -> eyre::Result<Vec<Token>> {
    let mut tokens = Vec::new();

    for (param, value) in params.into_iter() {
        let mut token = if lenient {
            LenientTokenizer::tokenize(param, value)
        } else {
            StrictTokenizer::tokenize(param, value)
        };
        if token.is_err() && value.starts_with("0x") {
            match param {
                ParamType::FixedBytes(32) => {
                    if value.len() < 66 {
                        let padded_value = [value, &"0".repeat(66 - value.len())].concat();
                        token = if lenient {
                            LenientTokenizer::tokenize(param, &padded_value)
                        } else {
                            StrictTokenizer::tokenize(param, &padded_value)
                        };
                    }
                }
                ParamType::Uint(_) => {
                    // try again if value is hex
                    if let Ok(value) = U256::from_str(value).map(|v| v.to_string()) {
                        token = if lenient {
                            LenientTokenizer::tokenize(param, &value)
                        } else {
                            StrictTokenizer::tokenize(param, &value)
                        };
                    }
                }
                // TODO: Not sure what to do here. Put the no effect in for now, but that is not
                // ideal. We could attempt massage for every value type?
                _ => {}
            }
        }

        let token = token.map(sanitize_token).wrap_err_with(|| {
            format!("Failed to parse `{value}`, expected value of type: {param}")
        })?;
        tokens.push(token);
    }
    Ok(tokens)
}

pub fn sanitize_token(token: Token) -> Token {
    match token {
        Token::Array(tokens) => {
            let mut sanitized = Vec::with_capacity(tokens.len());
            for token in tokens {
                let token = match token {
                    Token::String(val) => {
                        let val = match val.as_str() {
                            // this is supposed to be an empty string
                            "\"\"" | "''" => String::new(),
                            _ => val,
                        };
                        Token::String(val)
                    }
                    _ => sanitize_token(token),
                };
                sanitized.push(token)
            }
            Token::Array(sanitized)
        }
        _ => token,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use ethers::types::H160;
    use ethers_core::types::H256;

    const ADDRESS: &str = "0x4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38";
    const TRANSFER_SIG: &str = "Transfer(address indexed,address indexed,uint256)";
    const TRANSFER_TOPIC: &str =
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

    #[test]
    fn test_build_filter_basic() {
        let from_block = Some(BlockNumber::from(1337));
        let to_block = Some(BlockNumber::Latest);
        let address = Address::from_str(ADDRESS).ok();
        let expected = Filter {
            block_option: FilterBlockOption::Range { from_block, to_block },
            address: Some(ValueOrArray::Value(address.unwrap())),
            topics: [None, None, None, None],
        };
        let filter = build_filter(from_block, to_block, address, None, vec![]).unwrap();
        assert_eq!(filter, expected)
    }

    #[test]
    fn test_build_filter_sig() {
        let expected = Filter {
            block_option: FilterBlockOption::Range { from_block: None, to_block: None },
            address: None,
            topics: [Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()), None, None, None],
        };
        let filter =
            build_filter(None, None, None, Some(TRANSFER_SIG.to_string()), vec![]).unwrap();
        assert_eq!(filter, expected)
    }

    #[test]
    fn test_build_filter_mismatch() {
        let expected = Filter {
            block_option: FilterBlockOption::Range { from_block: None, to_block: None },
            address: None,
            topics: [Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()), None, None, None],
        };
        let filter = build_filter(
            None,
            None,
            None,
            Some("Swap(address indexed from, address indexed to, uint256 value)".to_string()), // Change signature, should result in error
            vec![],
        )
        .unwrap();
        assert_ne!(filter, expected)
    }

    #[test]
    fn test_build_filter_sig_with_arguments() {
        let expected = Filter {
            block_option: FilterBlockOption::Range { from_block: None, to_block: None },
            address: None,
            topics: [
                Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()),
                Some(H160::from_str(ADDRESS).unwrap().into()),
                None,
                None,
            ],
        };
        let filter = build_filter(
            None,
            None,
            None,
            Some(TRANSFER_SIG.to_string()),
            vec![ADDRESS.to_string()],
        )
        .unwrap();
        assert_eq!(filter, expected)
    }

    #[test]
    fn test_build_filter_sig_with_skipped_arguments() {
        let expected = Filter {
            block_option: FilterBlockOption::Range { from_block: None, to_block: None },
            address: None,
            topics: [
                Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()),
                None,
                Some(H160::from_str(ADDRESS).unwrap().into()),
                None,
            ],
        };
        let filter = build_filter(
            None,
            None,
            None,
            Some(TRANSFER_SIG.to_string()),
            vec![String::new(), ADDRESS.to_string()],
        )
        .unwrap();
        assert_eq!(filter, expected)
    }

    #[test]
    fn test_build_filter_with_topics() {
        let expected = Filter {
            block_option: FilterBlockOption::Range { from_block: None, to_block: None },
            address: None,
            topics: [
                Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()),
                Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()),
                None,
                None,
            ],
        };
        let filter = build_filter(
            None,
            None,
            None,
            Some(TRANSFER_TOPIC.to_string()),
            vec![TRANSFER_TOPIC.to_string()],
        )
        .unwrap();

        assert_eq!(filter, expected)
    }

    #[test]
    fn test_build_filter_with_skipped_topic() {
        let expected = Filter {
            block_option: FilterBlockOption::Range { from_block: None, to_block: None },
            address: None,
            topics: [
                Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()),
                None,
                Some(H256::from_str(TRANSFER_TOPIC).unwrap().into()),
                None,
            ],
        };
        let filter = build_filter(
            None,
            None,
            None,
            Some(TRANSFER_TOPIC.to_string()),
            vec![String::new(), TRANSFER_TOPIC.to_string()],
        )
        .unwrap();

        assert_eq!(filter, expected)
    }

    #[test]
    fn test_build_filter_sig_with_mismatched_argument() {
        let err = build_filter(
            None,
            None,
            None,
            Some(TRANSFER_SIG.to_string()),
            vec!["1234".to_string()],
        )
        .err()
        .unwrap()
        .to_string();

        assert_eq!(err, "Failed to parse `1234`, expected value of type: address");
    }

    #[test]
    fn test_build_filter_with_invalid_sig_or_topic() {
        let err = build_filter(None, None, None, Some("asdasdasd".to_string()), vec![])
            .err()
            .unwrap()
            .to_string();

        assert_eq!(err, "Invalid character 's' at position 1");
    }

    #[test]
    fn test_build_filter_with_invalid_sig_or_topic_hex() {
        let err = build_filter(None, None, None, Some(ADDRESS.to_string()), vec![])
            .err()
            .unwrap()
            .to_string();

        assert_eq!(err, "Invalid input length");
    }

    #[test]
    fn test_build_filter_with_invalid_topic() {
        let err = build_filter(
            None,
            None,
            None,
            Some(TRANSFER_TOPIC.to_string()),
            vec!["1234".to_string()],
        )
        .err()
        .unwrap()
        .to_string();

        assert_eq!(err, "Invalid input length");
    }
}
