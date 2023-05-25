// cast estimate subcommands
use crate::{
    opts::EthereumOpts,
    utils::{self},
};
use cast::Cast;
use clap::Parser;
use ethers::{
    abi::{Address, Topic},
    providers::Middleware,
    types::{BlockId, BlockNumber, Filter, FilterBlockOption, NameOrAddress, ValueOrArray, H256},
};

use foundry_common::abi::{get_event, parse_tokens};
use foundry_config::Config;

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
        let from_block = match from_block {
            Some(block) => match block {
                BlockId::Number(block_number) => Some(block_number),
                BlockId::Hash(hash) => {
                    let block = provider.get_block(hash).await?;
                    Some(BlockNumber::from(block.unwrap().number.unwrap()))
                }
            },
            None => None,
        };
        let to_block = match to_block {
            Some(block) => match block {
                BlockId::Number(block_number) => Some(block_number),
                BlockId::Hash(hash) => {
                    let block = provider.get_block(hash).await?;
                    Some(BlockNumber::from(block.unwrap().number.unwrap()))
                }
            },
            None => None,
        };

        let cast = Cast::new(&provider);

        let filter =
            build_filter(from_block, to_block, address, sig_or_topic, topics_or_args).unwrap();

        let logs = cast.filter_logs(filter, json).await?;

        println!("{}", logs);

        Ok(())
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

    let mut topics = match sig_or_topic {
        Some(sig_or_topic) => match get_event(sig_or_topic.as_str()) {
            Ok(event) => {
                let args = topics_or_args.iter().map(|arg| arg.as_str()).collect::<Vec<_>>();

                let indexed_inputs = event
                    .inputs
                    .iter()
                    .zip(args)
                    .filter(|(input, _)| input.indexed)
                    .map(|(input, arg)| (&input.kind, arg))
                    .collect::<Vec<_>>();

                let indexed_tokens = parse_tokens(indexed_inputs, true)?;

                let token0 = indexed_tokens.get(0);
                let token1 = indexed_tokens.get(1);
                let token2 = indexed_tokens.get(2);

                let raw = match (token0, token1, token2) {
                    (Some(token0), Some(token1), Some(token2)) => ethers::abi::RawTopicFilter {
                        topic0: Topic::This(token0.clone()),
                        topic1: Topic::This(token1.clone()),
                        topic2: Topic::This(token2.clone()),
                    },
                    (Some(token0), Some(token1), None) => ethers::abi::RawTopicFilter {
                        topic0: Topic::This(token0.clone()),
                        topic1: Topic::This(token1.clone()),
                        topic2: Topic::Any,
                    },
                    (Some(token0), None, None) => ethers::abi::RawTopicFilter {
                        topic0: Topic::This(token0.clone()),
                        topic1: Topic::Any,
                        topic2: Topic::Any,
                    },
                    (None, None, None) => ethers::abi::RawTopicFilter {
                        topic0: Topic::Any,
                        topic1: Topic::Any,
                        topic2: Topic::Any,
                    },
                    _ => panic!("Invalid number of indexed arguments"),
                };

                let filter = event.filter(raw).unwrap();
                [filter.topic0, filter.topic1, filter.topic2, filter.topic3]
                    .iter()
                    .map(|topic| match topic {
                        Topic::This(topic) => Some(ValueOrArray::Value(Some(*topic))),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            }
            Err(_) => {
                let mut topics = topics_or_args;
                topics.reverse();
                topics.push(sig_or_topic);
                topics.reverse();

                topics
                    .into_iter()
                    .map(|topic_str| {
                        Some(ValueOrArray::Value(Some(H256::from_str(topic_str.as_str()).unwrap())))
                    })
                    .collect::<Vec<_>>()
            }
        },
        None => Vec::new(),
    };

    topics.resize(4, None);

    let filter = Filter {
        block_option,
        address: address.map(ValueOrArray::Value),
        topics: [topics[0].clone(), topics[1].clone(), topics[2].clone(), topics[3].clone()],
    };

    Ok(filter)
}
