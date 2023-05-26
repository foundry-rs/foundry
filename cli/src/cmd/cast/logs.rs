// cast estimate subcommands
use crate::{
    opts::EthereumOpts,
    utils::{self},
};
use cast::Cast;
use clap::Parser;
use ethers::{
    abi::{Address, RawTopicFilter, Topic, TopicFilter},
    providers::Middleware,
    types::{BlockId, BlockNumber, Filter, FilterBlockOption, NameOrAddress, ValueOrArray, H256},
};

use foundry_common::abi::{get_event, parse_tokens};
use foundry_config::Config;

use itertools::Itertools;

use std::str::FromStr;

/// CLI arguments for `cast access-list`.
#[derive(Debug, Parser)]
pub struct LogsArgs {
    #[clap(flatten)]
    eth: EthereumOpts,
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

    /// Print the logs as JSON.
    #[clap(long, short, help_heading = "Display options")]
    json: bool,
}

impl LogsArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let LogsArgs {
            from_block, to_block, address, topics_or_args, sig_or_topic, json, eth, ..
        } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;

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

        let from_block = get_block_number(&provider, from_block).await?;
        let to_block = get_block_number(&provider, to_block).await?;

        let cast = Cast::new(&provider);

        let filter = build_filter(from_block, to_block, address, sig_or_topic, topics_or_args)?;

        let logs = cast.filter_logs(filter, json).await?;

        println!("{}", logs);

        Ok(())
    }
}

async fn get_block_number<M: Middleware>(
    provider: M,
    block: Option<BlockId>,
) -> Result<Option<BlockNumber>, eyre::Error>
where
    M::Error: 'static,
{
    match block {
        Some(block) => match block {
            BlockId::Number(block_number) => Ok(Some(block_number)),
            BlockId::Hash(hash) => {
                let block = provider.get_block(hash).await?;
                Ok(block.map(|block| block.number.unwrap()).map(BlockNumber::from))
            }
        },
        None => Ok(None),
    }
}

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
        Some(sig_or_topic) => match get_event(sig_or_topic.as_str()) {
            Ok(event) => {
                let args = topics_or_args.iter().map(|arg| arg.as_str()).collect::<Vec<_>>();

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
                let indexed_tokens = parse_tokens(
                    with_args.clone().into_iter().map(|(_, p)| p).collect::<Vec<_>>(),
                    true,
                )?;

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
                event.filter(raw)?
            }
            Err(_) => {
                let mut topics = Vec::new();
                topics.push(Some(H256::from_str(&sig_or_topic)?));
                for topic in topics_or_args {
                    topics.push(Some(H256::from_str(&topic)?));
                }

                topics.resize(4, None);

                TopicFilter {
                    topic0: topics[0].map_or(Topic::Any, Topic::This),
                    topic1: topics[1].map_or(Topic::Any, Topic::This),
                    topic2: topics[2].map_or(Topic::Any, Topic::This),
                    topic3: topics[3].map_or(Topic::Any, Topic::This),
                }
            }
        },
        None => TopicFilter::default(),
    };

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

#[cfg(test)]
mod tests {
    use ethers::types::H160;

    use super::*;

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
            vec!["".to_string(), ADDRESS.to_string()],
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
