//! Adaptive gas-envelope sampler for invariant fuzzing.
//!
//! Tracks observed `gas_used` per `(target, selector)` and samples per-call
//! `tx.gas_limit` biased to the "edge of feasibility" where OOG-induced bugs
//! become reachable. Calibration-free: observations accumulate online.
//!
//! Distribution per call (when an observation exists):
//!
//! | Bucket | Range                            | Purpose                  |
//! |--------|----------------------------------|--------------------------|
//! | 15%    | `None` (natural)                 | Keep calibration current |
//! | 80%    | razor: `[~0.5·max, max)`, k=4    | Hairline OOG band        |
//! | 5%     | `[max/10, max·3/10)`             | DoS / griefing           |

use alloy_primitives::{Address, Selector};
use parking_lot::RwLock;
use rand::Rng;
use std::{collections::HashMap, sync::Arc};

/// Floor for sampled gas — well above EVM intrinsic to avoid tx-validation noise.
const MIN_SAMPLED_GAS: u64 = 50_000;

/// Skip sampling for cheap selectors (below the OOG-corruption regime).
const MIN_OBSERVED_FOR_SAMPLING: u64 = 100_000;

/// Running max `gas_used` per `(target, selector)`. Cheaply cloneable.
#[derive(Clone, Debug, Default)]
pub struct GasObservations {
    inner: Arc<RwLock<HashMap<(Address, Selector), u64>>>,
}

impl GasObservations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `gas_used`; keeps the running max.
    pub fn record(&self, target: Address, selector: Selector, gas_used: u64) {
        if gas_used == 0 {
            return;
        }
        let mut w = self.inner.write();
        let entry = w.entry((target, selector)).or_insert(0);
        if gas_used > *entry {
            *entry = gas_used;
        }
    }

    pub fn max_for(&self, target: Address, selector: Selector) -> Option<u64> {
        self.inner.read().get(&(target, selector)).copied()
    }
}

/// Sample a per-call `gas_limit`. Returns `None` to use the executor's
/// natural limit (no observation yet, or the natural-gas bucket was rolled).
/// Sampled values are clamped to `block_cap`.
pub fn sample_gas_limit<R: Rng + ?Sized>(
    observations: &GasObservations,
    target: Address,
    selector: Selector,
    block_cap: u64,
    rng: &mut R,
) -> Option<u64> {
    let max = observations.max_for(target, selector)?;
    if max < MIN_OBSERVED_FOR_SAMPLING {
        return None;
    }

    let bucket: u8 = rng.random_range(0..100);
    let raw = match bucket {
        0..15 => return None,
        15..95 => {
            // Razor: power-law biased toward `max` (~32% within 1%, ~51% within 7%).
            let u: f64 = rng.random_range(0.0..1.0);
            let bite = ((max as f64) * u.powi(4)) as u64;
            let lo = max / 2;
            max.saturating_sub(bite).max(lo)
        }
        _ => {
            // 10–30% of max — DoS / griefing band.
            let lo = (max / 10).max(MIN_SAMPLED_GAS);
            let hi = (max * 3 / 10).max(lo + 1);
            rng.random_range(lo..hi)
        }
    };

    Some(raw.max(MIN_SAMPLED_GAS).min(block_cap))
}

/// Sample a per-call `tx.gasprice`. Targets bugs that branch on gasprice
/// (refund accounting, gas-aware token logic). Calibration-free.
///
/// | Bucket | Range              | Purpose                          |
/// |--------|--------------------|----------------------------------|
/// | 40%    | `0`                | Baseline / refund accounting     |
/// | 40%    | `[1, 1e10)` wei    | Low — typical L2 fee (~0–10 gwei)|
/// | 20%    | `[1e10, 1e12)` wei | High — congested L1 band         |
pub fn sample_gas_price<R: Rng + ?Sized>(rng: &mut R) -> u128 {
    let bucket: u8 = rng.random_range(0..100);
    match bucket {
        0..40 => 0,
        40..80 => rng.random_range(1u128..10_000_000_000u128),
        _ => rng.random_range(10_000_000_000u128..1_000_000_000_000u128),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use rand::{SeedableRng, rngs::StdRng};

    fn target() -> Address {
        address!("00000000000000000000000000000000000000aa")
    }

    fn sel() -> Selector {
        Selector::from([0xde, 0xad, 0xbe, 0xef])
    }

    #[test]
    fn returns_none_without_observations() {
        let obs = GasObservations::new();
        let mut rng = StdRng::seed_from_u64(42);
        assert!(sample_gas_limit(&obs, target(), sel(), 30_000_000, &mut rng).is_none());
    }

    #[test]
    fn records_running_max() {
        let obs = GasObservations::new();
        obs.record(target(), sel(), 1_000);
        obs.record(target(), sel(), 5_000);
        obs.record(target(), sel(), 2_000);
        assert_eq!(obs.max_for(target(), sel()), Some(5_000));
    }

    #[test]
    fn samples_stay_within_block_cap() {
        let obs = GasObservations::new();
        obs.record(target(), sel(), 4_730_082); // baseline from gist
        let mut rng = StdRng::seed_from_u64(1);
        let block_cap = 30_000_000_u64;
        for _ in 0..10_000 {
            if let Some(g) = sample_gas_limit(&obs, target(), sel(), block_cap, &mut rng) {
                assert!(g >= MIN_SAMPLED_GAS, "gas {g} below intrinsic floor");
                assert!(g <= block_cap, "gas {g} exceeds block cap");
            }
        }
    }

    #[test]
    fn gas_price_distribution_matches_buckets() {
        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let mut zero = 0;
        let mut low = 0;
        let mut high = 0;
        for _ in 0..10_000 {
            let p = sample_gas_price(&mut rng);
            if p == 0 {
                zero += 1;
            } else if p < 10_000_000_000 {
                low += 1;
            } else if p < 1_000_000_000_000 {
                high += 1;
            } else {
                panic!("sampled gas_price {p} exceeded upper bucket cap");
            }
        }
        // Expected ~40/40/20 — allow a generous 5-pp tolerance.
        assert!((35..=45).contains(&(zero / 100)), "zero bucket: {zero}/10000");
        assert!((35..=45).contains(&(low / 100)), "low bucket: {low}/10000");
        assert!((15..=25).contains(&(high / 100)), "high bucket: {high}/10000");
    }

    #[test]
    fn distribution_hits_edge_band() {
        // The 65% threshold from the gist (3_074_553 / 4_730_082 ≈ 0.65) must be
        // reachable by the sampler — this is the band where the Tempo bug manifests.
        let obs = GasObservations::new();
        let max = 4_730_082_u64;
        obs.record(target(), sel(), max);
        let mut rng = StdRng::seed_from_u64(7);
        let mut hits_edge = 0;
        let mut total = 0;
        for _ in 0..10_000 {
            if let Some(g) = sample_gas_limit(&obs, target(), sel(), 30_000_000, &mut rng) {
                total += 1;
                // 60-70% of max — the corruption window in the gist
                if g >= max * 60 / 100 && g <= max * 70 / 100 {
                    hits_edge += 1;
                }
            }
        }
        assert!(total > 0);
        // Edge band is ~10% of max. With 40% of samples in the [50%, 110%] range
        // (uniform), the [60%, 70%] sub-band should get ~6-7% of samples.
        let pct = hits_edge * 100 / total;
        assert!(pct >= 3, "edge band hit rate too low: {pct}% ({hits_edge}/{total})");
    }
}
