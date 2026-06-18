//! Per-call `tx.gas_limit` sampler for invariant fuzzing.
//!
//! Defaults the per-call gas limit to the EIP-7825 transaction cap (`2^24` =
//! 16_777_216) and samples uniformly in `[2^24, 2^25)`. Calibration-free, no
//! per-(target, selector) state, no observations map.
//!
//! Lifting the executor's natural limit out of the call path varies the gas
//! envelope across the EIP-7825 cap on every call, exercising gas-conditional
//! EVM dispatch (refund accounting, EIP-150 1/64 retention, OOG dispatch at
//! the cap) that a fixed natural limit cannot.

use rand::Rng;

/// EIP-7825 transaction gas-limit cap.
pub const TX_GAS_CAP: u64 = 1 << 24;

/// Sample a per-call `tx.gas_limit` uniformly in `[TX_GAS_CAP, TX_GAS_CAP * 2)`.
pub fn sample_gas_limit<R: Rng + ?Sized>(rng: &mut R) -> u64 {
    rng.random_range(TX_GAS_CAP..(TX_GAS_CAP * 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn sampled_gas_limit_stays_in_range() {
        let mut rng = StdRng::seed_from_u64(0xAA);
        for _ in 0..10_000 {
            let g = sample_gas_limit(&mut rng);
            assert!(
                (TX_GAS_CAP..TX_GAS_CAP * 2).contains(&g),
                "sampled gas_limit {g} outside [2^24, 2^25)",
            );
        }
    }
}
