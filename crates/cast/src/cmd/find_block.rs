use crate::Cast;
use alloy_provider::Provider;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::print_scalar,
    opts::RpcOpts,
    utils::{self, LoadConfig},
};
use futures::join;

/// CLI arguments for `cast find-block`.
#[derive(Clone, Debug, Parser)]
pub struct FindBlockArgs {
    /// The UNIX timestamp to search for, in seconds.
    timestamp: u64,

    #[command(flatten)]
    rpc: RpcOpts,
}

fn interpolate_block(
    low_block: u64,
    low_timestamp: u64,
    high_block: u64,
    high_timestamp: u64,
    target_timestamp: u64,
) -> u64 {
    let block_range = high_block - low_block;
    let midpoint = high_block - block_range / 2;
    if high_timestamp <= low_timestamp {
        return midpoint;
    }

    let timestamp_offset = target_timestamp - low_timestamp;
    let timestamp_range = high_timestamp - low_timestamp;
    let block_offset =
        u128::from(timestamp_offset) * u128::from(block_range) / u128::from(timestamp_range);
    (low_block + block_offset as u64).clamp(low_block + 1, high_block - 1)
}

impl FindBlockArgs {
    pub async fn run(self) -> Result<()> {
        let Self { timestamp, rpc } = self;

        let ts_target = timestamp;
        let config = rpc.load_config()?;
        let provider = utils::get_provider(&config)?;

        let last_block_num = provider.get_block_number().await?;
        let cast_provider = Cast::new(provider);

        let res = join!(cast_provider.timestamp(last_block_num), cast_provider.timestamp(1));
        let ts_block_latest: u64 = res.0?.to();
        let ts_block_1: u64 = res.1?.to();

        let block_num = if ts_block_latest < ts_target {
            // If the most recent block's timestamp is below the target, return it
            last_block_num
        } else if ts_block_1 > ts_target {
            // If the target timestamp is below block 1's timestamp, return that
            1
        } else {
            // Otherwise, find the block that is closest to the timestamp
            let mut low_block = 1_u64; // block 0 has a timestamp of 0: https://github.com/ethereum/go-ethereum/issues/17042#issuecomment-559414137
            let mut low_timestamp = ts_block_1;
            let mut high_block = last_block_num;
            let mut high_timestamp = ts_block_latest;
            // Limit interpolation to the range's binary search depth so irregular chains retain
            // logarithmic worst-case behavior.
            let mut interpolation_budget =
                u64::BITS - last_block_num.saturating_sub(1).leading_zeros();
            loop {
                let block_range = high_block - low_block;
                if block_range == 0 {
                    break low_block;
                }
                if block_range == 1 {
                    // Round to the higher block when the timestamp is equidistant.
                    let high_diff = high_timestamp - ts_target;
                    let low_diff = ts_target - low_timestamp;
                    break if low_diff < high_diff { low_block } else { high_block };
                }

                let midpoint = high_block - block_range / 2;
                let next_block = if interpolation_budget == 0 {
                    midpoint
                } else {
                    interpolation_budget -= 1;
                    interpolate_block(
                        low_block,
                        low_timestamp,
                        high_block,
                        high_timestamp,
                        ts_target,
                    )
                };
                let next_timestamp = cast_provider.timestamp(next_block).await?.to::<u64>();

                if next_timestamp == ts_target {
                    break next_block;
                }
                if next_timestamp < ts_target {
                    low_block = next_block;
                    low_timestamp = next_timestamp;
                } else {
                    high_block = next_block;
                    high_timestamp = next_timestamp;
                }
            }
        };
        print_scalar(block_num)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::interpolate_block;

    #[test]
    fn interpolates_block_from_timestamps() {
        assert_eq!(interpolate_block(1, 100, 11, 200, 150), 6);
    }

    #[test]
    fn keeps_interpolation_inside_search_bounds() {
        assert_eq!(interpolate_block(1, 100, 11, 200, 100), 2);
        assert_eq!(interpolate_block(1, 100, 11, 200, 200), 10);
    }

    #[test]
    fn interpolates_without_overflow() {
        assert_eq!(interpolate_block(0, 0, u64::MAX, u64::MAX, u64::MAX / 2), u64::MAX / 2);
    }

    #[test]
    fn uses_midpoint_for_equal_timestamps() {
        assert_eq!(interpolate_block(1, 100, 10, 100, 100), 6);
    }
}
