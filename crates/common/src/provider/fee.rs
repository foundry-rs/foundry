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

/// Gas used ratio below which a block is treated as near-empty and its reward is
/// dropped from sampling, so one-off tips in otherwise idle blocks do not
/// dominate the cross-block median on low-traffic chains. Local heuristic: there
/// is no protocol-defined utilization threshold for priority-fee estimation.
const MIN_GAS_USED_RATIO: f64 = 0.1;

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

    let max_priority_fee_per_gas = estimate_priority_fee(
        fee_history.reward.as_deref().unwrap_or_default(),
        &fee_history.gas_used_ratio,
    );

    let (num, den) = preset.base_fee_multiplier();
    let max_fee_per_gas = base_fee_per_gas
        .checked_mul(num)
        .map_or(u128::MAX, |scaled| scaled / den)
        .saturating_add(max_priority_fee_per_gas);

    Ok(ResolvedEip1559Fees { max_fee_per_gas, max_priority_fee_per_gas, base_fee_per_gas })
}

/// Applies, in order, the optional `browser_suggested_tip`, `with_gas_price` and
/// `priority_gas_price` overrides to estimated EIP-1559 fees.
///
/// `with_gas_price` overrides only `maxFeePerGas` and `priority_gas_price` only
/// `maxPriorityFeePerGas`. Errors if the result has `maxPriorityFeePerGas` above
/// `maxFeePerGas`.
pub fn resolve_broadcast_eip1559_fees(
    mut fees: ResolvedEip1559Fees,
    with_gas_price: Option<u128>,
    priority_gas_price: Option<u128>,
    browser_suggested_tip: Option<u128>,
) -> Result<ResolvedEip1559Fees> {
    // Raise both caps by the same delta so `maxFeePerGas` keeps its buffer above
    // the higher tip.
    if let Some(suggested_tip) = browser_suggested_tip
        && suggested_tip > fees.max_priority_fee_per_gas
    {
        let delta = suggested_tip - fees.max_priority_fee_per_gas;
        fees.max_fee_per_gas = fees.max_fee_per_gas.saturating_add(delta);
        fees.max_priority_fee_per_gas = suggested_tip;
    }

    if let Some(max_fee_per_gas) = with_gas_price {
        fees.max_fee_per_gas = max_fee_per_gas;
    }

    if let Some(max_priority_fee_per_gas) = priority_gas_price {
        fees.max_priority_fee_per_gas = max_priority_fee_per_gas;
    }

    if fees.max_priority_fee_per_gas > fees.max_fee_per_gas {
        eyre::bail!(
            "maxPriorityFeePerGas ({}) cannot be higher than maxFeePerGas ({})",
            fees.max_priority_fee_per_gas,
            fees.max_fee_per_gas,
        );
    }

    Ok(fees)
}

/// Estimates the priority fee as the median of the per-block rewards, ignoring
/// zero rewards and rewards from blocks below [`MIN_GAS_USED_RATIO`], and never
/// returning less than [`MIN_PRIORITY_FEE`].
fn estimate_priority_fee(rewards: &[Vec<u128>], gas_used_ratio: &[f64]) -> u128 {
    let mut rewards = rewards
        .iter()
        .zip(gas_used_ratio)
        .filter(|(_, ratio)| ratio.is_finite() && ratio.clamp(0.0, 1.0) >= MIN_GAS_USED_RATIO)
        .filter_map(|(reward, _)| reward.first().copied())
        .filter(|reward| *reward > 0)
        .collect::<Vec<_>>();
    if rewards.is_empty() {
        return MIN_PRIORITY_FEE;
    }

    rewards.sort_unstable();

    let n = rewards.len();
    // `midpoint` avoids overflow when averaging the two middle values.
    let median =
        if n % 2 == 0 { rewards[n / 2 - 1].midpoint(rewards[n / 2]) } else { rewards[n / 2] };

    std::cmp::max(median, MIN_PRIORITY_FEE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gas used ratio above [`MIN_GAS_USED_RATIO`].
    const BUSY: f64 = 0.5;

    #[test]
    fn priority_fee_median_of_busy_blocks() {
        // Empty rewards -> minimum priority fee.
        assert_eq!(estimate_priority_fee(&[], &[]), MIN_PRIORITY_FEE);
        assert_eq!(estimate_priority_fee(&[vec![0], vec![0]], &[BUSY, BUSY]), MIN_PRIORITY_FEE);

        // Median of non-zero rewards from busy blocks.
        assert_eq!(estimate_priority_fee(&[vec![1], vec![3], vec![5]], &[BUSY, BUSY, BUSY]), 3);
        assert_eq!(estimate_priority_fee(&[vec![2], vec![4]], &[BUSY, BUSY]), 3);
    }

    #[test]
    fn priority_fee_ignores_near_empty_blocks() {
        // Rewards from blocks below MIN_GAS_USED_RATIO are dropped.
        let rewards = vec![vec![0u128], vec![41_000_000_000_000u128], vec![0u128]];
        let ratios = vec![0.0, 0.001, 0.0];
        assert_eq!(estimate_priority_fee(&rewards, &ratios), MIN_PRIORITY_FEE);

        // Only busy blocks are sampled -> median of [2, 3] = 2.
        let rewards = vec![vec![500u128], vec![2u128], vec![800u128], vec![3u128], vec![999u128]];
        let ratios = vec![0.01, BUSY, 0.03, 0.7, 0.02];
        assert_eq!(estimate_priority_fee(&rewards, &ratios), 2);

        // The threshold is inclusive; NaN/infinite ratios are dropped.
        assert_eq!(estimate_priority_fee(&[vec![10], vec![20]], &[0.1, 0.09]), 10);
        assert_eq!(
            estimate_priority_fee(&[vec![5], vec![6]], &[f64::NAN, f64::INFINITY]),
            MIN_PRIORITY_FEE
        );
    }

    #[test]
    fn priority_fee_tolerates_length_mismatch() {
        // `zip` bounds the sample to the shorter slice.
        assert_eq!(estimate_priority_fee(&[vec![7], vec![9], vec![11]], &[BUSY]), 7);
        assert_eq!(estimate_priority_fee(&[vec![7]], &[BUSY, BUSY, BUSY]), 7);
    }

    /// Drives the public async estimator so `gas_used_ratio` wiring is covered.
    #[tokio::test]
    async fn estimate_filters_near_empty_block_outliers() {
        use alloy_provider::{ProviderBuilder, mock::Asserter};
        use alloy_rpc_types::FeeHistory;

        let base = 20_000_000_000u128; // 20 gwei
        let big = 41_000_000_000_000u128; // outlier tip in near-empty blocks
        let fee_history = FeeHistory {
            base_fee_per_gas: vec![base; 11],
            gas_used_ratio: vec![0.001; 10],
            base_fee_per_blob_gas: vec![1; 11],
            blob_gas_used_ratio: vec![0.0; 10],
            oldest_block: 1,
            reward: Some(vec![
                vec![0u128],
                vec![big],
                vec![0u128],
                vec![big],
                vec![0u128],
                vec![big],
                vec![0u128],
                vec![0u128],
                vec![0u128],
                vec![0u128],
            ]),
        };

        let asserter = Asserter::new();
        asserter.push_success(&fee_history);
        let provider = ProviderBuilder::new_with_network::<alloy_network::Ethereum>()
            .connect_mocked_client(asserter);

        let fees =
            estimate_eip1559_fees(&provider, Eip1559FeeEstimatePreset::Market).await.unwrap();

        // Near-empty outliers are dropped -> min priority, max_fee = 2 * base + min.
        assert_eq!(fees.base_fee_per_gas, base);
        assert_eq!(fees.max_priority_fee_per_gas, MIN_PRIORITY_FEE);
        assert_eq!(fees.max_fee_per_gas, 40_000_000_001);
    }

    #[test]
    fn tempo_scenario_yields_expected_max_fee() {
        // Outlier tip in a near-empty block -> Market max_fee = 2 * base + min.
        let base_fee = 20_000_000_000u128;
        let rewards = vec![vec![0u128], vec![41_000_000_000_000u128], vec![0u128]];
        let ratios = vec![BUSY, 0.001, BUSY];

        let priority = estimate_priority_fee(&rewards, &ratios);
        let (num, den) = Eip1559FeeEstimatePreset::Market.base_fee_multiplier();
        let max_fee = base_fee.saturating_mul(num) / den + priority;

        assert_eq!(priority, MIN_PRIORITY_FEE);
        assert_eq!(max_fee, 40_000_000_001);
    }

    fn fees(max: u128, priority: u128) -> ResolvedEip1559Fees {
        ResolvedEip1559Fees {
            max_fee_per_gas: max,
            max_priority_fee_per_gas: priority,
            base_fee_per_gas: 100,
        }
    }

    #[test]
    fn resolve_overrides_each_field_independently() {
        // No overrides: passthrough.
        let r = resolve_broadcast_eip1559_fees(fees(300, 50), None, None, None).unwrap();
        assert_eq!((r.max_fee_per_gas, r.max_priority_fee_per_gas), (300, 50));

        // `--with-gas-price` overrides only max, `--priority-gas-price` only priority.
        let r = resolve_broadcast_eip1559_fees(fees(300, 50), Some(500), None, None).unwrap();
        assert_eq!((r.max_fee_per_gas, r.max_priority_fee_per_gas), (500, 50));
        let r = resolve_broadcast_eip1559_fees(fees(300, 50), None, Some(80), None).unwrap();
        assert_eq!((r.max_fee_per_gas, r.max_priority_fee_per_gas), (300, 80));

        // Invalid when the resulting priority exceeds max.
        let err = resolve_broadcast_eip1559_fees(fees(300, 50), None, Some(400), None).unwrap_err();
        assert!(err.to_string().contains("cannot be higher than maxFeePerGas"));
    }

    #[test]
    fn resolve_browser_tip_raises_both_caps_by_delta() {
        // Higher tip raises priority to the tip and max by the same delta.
        let r = resolve_broadcast_eip1559_fees(fees(300, 50), None, None, Some(120)).unwrap();
        assert_eq!((r.max_fee_per_gas, r.max_priority_fee_per_gas), (370, 120)); // 300 + (120 - 50)

        // Lower tip is ignored; max saturates instead of overflowing.
        let r = resolve_broadcast_eip1559_fees(fees(300, 50), None, None, Some(10)).unwrap();
        assert_eq!((r.max_fee_per_gas, r.max_priority_fee_per_gas), (300, 50));
        let r = resolve_broadcast_eip1559_fees(fees(u128::MAX, 50), None, None, Some(120)).unwrap();
        assert_eq!((r.max_fee_per_gas, r.max_priority_fee_per_gas), (u128::MAX, 120));
    }

    #[test]
    fn market_preset_max_fee_formula() {
        // Market preset: max_fee = base_fee * 2 + priority_fee.
        let preset = Eip1559FeeEstimatePreset::Market;
        let (num, den) = preset.base_fee_multiplier();
        let base_fee = 2_000_000_000u128; // 2 gwei
        let priority = estimate_priority_fee(&[vec![100], vec![300]], &[BUSY, BUSY]);
        let max_fee = base_fee.saturating_mul(num) / den + priority;
        assert_eq!(max_fee, base_fee * 2 + priority);
    }

    #[test]
    fn priority_fee_preserves_high_tips_on_busy_chains() {
        // Busy blocks keep the median of non-zero rewards, even when the tip is
        // far above the base fee (legitimate congestion is not clamped).
        let rewards = vec![vec![10_000_000_000u128], vec![30_000_000_000u128]];
        assert_eq!(estimate_priority_fee(&rewards, &[BUSY, BUSY]), 20_000_000_000);
    }
}
