//! cast find-block subcommand

use crate::{cmd::Cmd, utils::consume_config_rpc_url};
use cast::Cast;
use clap::Parser;
use ethers::prelude::*;
use eyre::Result;
use futures::{future::BoxFuture, join};

#[derive(Debug, Clone, Parser)]
pub struct FindBlockArgs {
    #[clap(help = "The UNIX timestamp to search for (in seconds)", value_name = "TIMESTAMP")]
    timestamp: u64,
    #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
    rpc_url: Option<String>,
}

impl Cmd for FindBlockArgs {
    type Output = BoxFuture<'static, Result<()>>;

    fn run(self) -> Result<Self::Output> {
        let FindBlockArgs { timestamp, rpc_url } = self;
        Ok(Box::pin(Self::query_block(timestamp, rpc_url)))
    }
}

impl FindBlockArgs {
    async fn query_block(timestamp: u64, rpc_url: Option<String>) -> Result<()> {
        let ts_target = U256::from(timestamp);
        let rpc_url = consume_config_rpc_url(rpc_url);

        let provider = Provider::try_from(rpc_url)?;
        let last_block_num = provider.get_block_number().await?;
        let cast_provider = Cast::new(provider);

        let res = join!(cast_provider.timestamp(last_block_num), cast_provider.timestamp(1));
        let ts_block_latest = res.0?;
        let ts_block_1 = res.1?;

        let block_num = if ts_block_latest.lt(&ts_target) {
            // If the most recent block's timestamp is below the target, return it
            last_block_num
        } else if ts_block_1.gt(&ts_target) {
            // If the target timestamp is below block 1's timestamp, return that
            U64::from(1_u64)
        } else {
            // Otherwise, find the block that is closest to the timestamp
            let mut low_block = U64::from(1_u64); // block 0 has a timestamp of 0: https://github.com/ethereum/go-ethereum/issues/17042#issuecomment-559414137
            let mut high_block = last_block_num;
            let mut matching_block: Option<U64> = None;
            while high_block.gt(&low_block) && matching_block.is_none() {
                // Get timestamp of middle block (this approach approach to avoids overflow)
                let high_minus_low_over_2 = high_block
                    .checked_sub(low_block)
                    .ok_or_else(|| eyre::eyre!("unexpected underflow"))
                    .unwrap()
                    .checked_div(U64::from(2_u64))
                    .unwrap();
                let mid_block = high_block.checked_sub(high_minus_low_over_2).unwrap();
                let ts_mid_block = cast_provider.timestamp(mid_block).await?;

                // Check if we've found a match or should keep searching
                if ts_mid_block.eq(&ts_target) {
                    matching_block = Some(mid_block)
                } else if high_block.checked_sub(low_block).unwrap().eq(&U64::from(1_u64)) {
                    // The target timestamp is in between these blocks. This rounds to the
                    // highest block if timestamp is equidistant between blocks
                    let res = join!(
                        cast_provider.timestamp(high_block),
                        cast_provider.timestamp(low_block)
                    );
                    let ts_high = res.0.unwrap();
                    let ts_low = res.1.unwrap();
                    let high_diff = ts_high.checked_sub(ts_target).unwrap();
                    let low_diff = ts_target.checked_sub(ts_low).unwrap();
                    let is_low = low_diff.lt(&high_diff);
                    matching_block = if is_low { Some(low_block) } else { Some(high_block) }
                } else if ts_mid_block.lt(&ts_target) {
                    low_block = mid_block;
                } else {
                    high_block = mid_block;
                }
            }
            matching_block.unwrap_or(low_block)
        };
        println!("{block_num}");

        Ok(())
    }
}
