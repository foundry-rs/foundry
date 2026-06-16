//! EIP-1559 fee estimation with configurable presets.
//!
//! Estimates `maxFeePerGas` and `maxPriorityFeePerGas` from `eth_feeHistory`,
//! where the chosen [`Eip1559FeeEstimatePreset`] selects the reward percentile
//! used for the priority fee and the multiplier applied to the base fee.

use alloy_consensus::BlockHeader;
use alloy_eips::BlockNumberOrTag;
use alloy_network::{BlockResponse, Network};
use alloy_provider::{Provider, utils::Eip1559Estimation};
use eyre::{Result, WrapErr};
use foundry_config::Eip1559FeeEstimatePreset;

/// The number of past blocks sampled from `eth_feeHistory` for fee estimation.
const FEE_HISTORY_BLOCKS: u64 = 10;

/// The minimum priority fee to provide, in wei.
const MIN_PRIORITY_FEE: u128 = 1;

/// The resolved EIP-1559 fees together with the base fee they were derived from.
///
/// The base fee is surfaced so callers can present a fee breakdown (max fee /
/// priority fee / base fee) instead of a single, easily-misread "gas price".
#[derive(Clone, Copy, Debug)]
pub struct ResolvedEip1559Fees {
    /// `maxFeePerGas`.
    pub max_fee_per_gas: u128,
    /// `maxPriorityFeePerGas`.
    pub max_priority_fee_per_gas: u128,
    /// The base fee of the latest block the estimate was derived from.
    pub base_fee_per_gas: u128,
}

impl ResolvedEip1559Fees {
    /// Returns the max fee and priority fee as an [`Eip1559Estimation`], dropping
    /// the base fee.
    pub const fn estimation(&self) -> Eip1559Estimation {
        Eip1559Estimation {
            max_fee_per_gas: self.max_fee_per_gas,
            max_priority_fee_per_gas: self.max_priority_fee_per_gas,
        }
    }
}

/// Estimates EIP-1559 fees for `provider` using the given `preset`.
///
/// `preset` controls the reward percentile sampled for the priority fee and the
/// base-fee multiplier used to build `maxFeePerGas`.
pub async fn estimate_eip1559_fees<P, N>(
    provider: &P,
    preset: Eip1559FeeEstimatePreset,
) -> Result<ResolvedEip1559Fees>
where
    P: Provider<N>,
    N: Network,
{
    let fee_history = provider
        .get_fee_history(
            FEE_HISTORY_BLOCKS,
            BlockNumberOrTag::Latest,
            &[preset.reward_percentile()],
        )
        .await
        .wrap_err("Failed to fetch fee history for EIP-1559 estimation")?;

    // Use the base fee of the latest mined block. If the fee history omits or
    // zeroes it, read it from the latest block header; if that is also absent the
    // chain does not support EIP-1559, so error instead of guessing.
    let base_fee_per_gas = match fee_history.latest_block_base_fee() {
        Some(base_fee) if base_fee != 0 => base_fee,
        _ => provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .wrap_err("Failed to fetch latest block for EIP-1559 base fee")?
            .ok_or_else(|| eyre::eyre!("Latest block not found"))?
            .header()
            .as_ref()
            .base_fee_per_gas()
            .ok_or_else(|| {
                eyre::eyre!(
                    "Chain does not appear to support EIP-1559; try adding --legacy to your command."
                )
            })?
            .into(),
    };

    let max_priority_fee_per_gas =
        estimate_priority_fee(fee_history.reward.as_deref().unwrap_or_default());

    let (num, den) = preset.base_fee_multiplier();
    let max_fee_per_gas = base_fee_per_gas
        .checked_mul(num)
        .map_or(u128::MAX, |scaled| scaled / den)
        .saturating_add(max_priority_fee_per_gas);

    Ok(ResolvedEip1559Fees { max_fee_per_gas, max_priority_fee_per_gas, base_fee_per_gas })
}

/// Estimates the priority fee as the median of the per-block sampled rewards,
/// ignoring zero rewards and never returning less than [`MIN_PRIORITY_FEE`].
fn estimate_priority_fee(rewards: &[Vec<u128>]) -> u128 {
    let mut rewards =
        rewards.iter().filter_map(|r| r.first()).filter(|r| **r > 0_u128).collect::<Vec<_>>();
    if rewards.is_empty() {
        return MIN_PRIORITY_FEE;
    }

    rewards.sort_unstable();

    let n = rewards.len();
    let median =
        if n % 2 == 0 { (*rewards[n / 2 - 1] + *rewards[n / 2]) / 2 } else { *rewards[n / 2] };

    std::cmp::max(median, MIN_PRIORITY_FEE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_fee_matches_alloy_semantics() {
        // Empty rewards -> minimum priority fee.
        assert_eq!(estimate_priority_fee(&[]), MIN_PRIORITY_FEE);
        assert_eq!(estimate_priority_fee(&[vec![0], vec![0]]), MIN_PRIORITY_FEE);

        // Median of non-zero rewards.
        assert_eq!(estimate_priority_fee(&[vec![1], vec![3], vec![5]]), 3);
        assert_eq!(estimate_priority_fee(&[vec![2], vec![4]]), 3);
    }

    #[test]
    fn market_preset_matches_alloy_default_formula() {
        // alloy default: max_fee = base_fee * 2 + priority_fee.
        let preset = Eip1559FeeEstimatePreset::Market;
        let (num, den) = preset.base_fee_multiplier();
        let base_fee = 2_000_000_000u128; // 2 gwei
        let priority = estimate_priority_fee(&[vec![100], vec![300]]);
        let max_fee = base_fee.saturating_mul(num) / den + priority;
        assert_eq!(max_fee, base_fee * 2 + priority);
    }

    /// The `market` preset must produce exactly what alloy's public default
    /// estimator produces, for the same `(base_fee, rewards)` inputs. This is the
    /// guarantee that switching to our resolver does not change default behavior.
    #[test]
    fn market_preset_matches_alloy_default_estimator() {
        let preset = Eip1559FeeEstimatePreset::Market;
        let (num, den) = preset.base_fee_multiplier();

        for base_fee in [0u128, 1, 1_000_000_000, 2_000_000_000, 137_000_000_000] {
            for rewards in [
                vec![],
                vec![vec![0u128]],
                vec![vec![100], vec![0], vec![300], vec![500]],
                vec![vec![2], vec![4]],
            ] {
                let priority = estimate_priority_fee(&rewards);
                let ours = Eip1559Estimation {
                    max_fee_per_gas: base_fee.saturating_mul(num) / den + priority,
                    max_priority_fee_per_gas: priority,
                };
                let alloy = alloy_provider::utils::eip1559_default_estimator(base_fee, &rewards);
                assert_eq!(ours, alloy, "mismatch for base_fee={base_fee}, rewards={rewards:?}");
            }
        }
    }
}
