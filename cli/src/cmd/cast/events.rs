// cast estimate subcommands
use crate::{opts::EthereumOpts, utils};
use clap::Parser;
use ethers::{
    abi::Address,
    providers::Middleware,
    types::{BlockNumber, Filter, NameOrAddress, TxHash, ValueOrArray},
};
use foundry_config::Config;

fn block_number_parser(s: &str) -> Result<BlockNumber, &'static str> {
    Ok(s.parse::<BlockNumber>().unwrap())
}

#[derive(Debug, Parser)]
pub struct EventsArgs {
    /// Source contract of the event.
    target: Vec<NameOrAddress>,

    #[clap(long)]
    topic0: Option<Vec<TxHash>>,
    #[clap(long)]
    topic1: Option<Vec<TxHash>>,
    #[clap(long)]
    topic2: Option<Vec<TxHash>>,
    #[clap(long)]
    topic3: Option<Vec<TxHash>>,

    #[clap(long, value_parser = block_number_parser)]
    from_block: Option<BlockNumber>,

    #[clap(long, value_parser = block_number_parser)]
    to_block: Option<BlockNumber>,

    #[clap(flatten)]
    eth: EthereumOpts,
}

impl EventsArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let EventsArgs { eth, target, topic0, topic1, topic2, topic3, from_block, to_block } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;

        let mut addresses = vec![];
        for addr in target {
            match addr {
                NameOrAddress::Name(name) => match name.parse::<Address>() {
                    Ok(resolved_addr) => addresses.push(resolved_addr),
                    Err(_) => {
                        let resolved_addr = provider.resolve_name(name.as_str()).await?;
                        addresses.push(resolved_addr);
                    }
                },
                NameOrAddress::Address(addr) => addresses.push(addr),
            }
        }

        let filter = Filter::new().address(ValueOrArray::Array(addresses));
        let filter = if let Some(topic0) = topic0 {
            filter.topic0(ValueOrArray::Array(topic0))
        } else {
            filter
        };
        let filter = if let Some(topic1) = topic1 {
            filter.topic1(ValueOrArray::Array(topic1))
        } else {
            filter
        };
        let filter = if let Some(topic2) = topic2 {
            filter.topic2(ValueOrArray::Array(topic2))
        } else {
            filter
        };
        let filter = if let Some(topic3) = topic3 {
            filter.topic3(ValueOrArray::Array(topic3))
        } else {
            filter
        };
        let filter =
            if let Some(from_block) = from_block { filter.from_block(from_block) } else { filter };
        let filter = if let Some(to_block) = to_block { filter.to_block(to_block) } else { filter };

        let logs = provider.get_logs(&filter).await?;

        for log in logs {
            println!("{:#?}", log);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::types::Address;
}
