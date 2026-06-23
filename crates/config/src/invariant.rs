//! Configuration for invariant testing

use crate::fuzz::{FuzzCorpusConfig, FuzzDictionaryConfig};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error, Visitor},
};
use std::{fmt, num::NonZeroUsize, path::PathBuf, str::FromStr};

/// Worker selection mode for invariant campaign sharding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvariantWorkers {
    /// Automatically derive invariant workers from the active `--jobs` / rayon thread pool.
    Auto,
    /// Explicit user override for invariant campaign sharding.
    Fixed(NonZeroUsize),
}

impl Default for InvariantWorkers {
    fn default() -> Self {
        Self::Fixed(NonZeroUsize::MIN)
    }
}

impl Serialize for InvariantWorkers {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Fixed(workers) => workers.get().serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for InvariantWorkers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(InvariantWorkersVisitor)
    }
}

impl FromStr for InvariantWorkers {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }

        let workers = value.parse::<usize>().map_err(|err| err.to_string())?;
        fixed_workers(workers)
    }
}

struct InvariantWorkersVisitor;

impl Visitor<'_> for InvariantWorkersVisitor {
    type Value = InvariantWorkers;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("`auto` or a positive integer worker count")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: Error,
    {
        value.parse().map_err(E::custom)
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        let workers = usize::try_from(value).map_err(E::custom)?;
        fixed_workers(workers).map_err(E::custom)
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: Error,
    {
        let workers =
            usize::try_from(value).map_err(|_| E::custom("invariant workers must be positive"))?;
        fixed_workers(workers).map_err(E::custom)
    }
}

fn fixed_workers(workers: usize) -> Result<InvariantWorkers, String> {
    NonZeroUsize::new(workers)
        .map(InvariantWorkers::Fixed)
        .ok_or_else(|| "invariant workers must be greater than 0".to_string())
}

/// Contains for invariant testing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantConfig {
    /// The number of runs that must execute for each invariant test group.
    pub runs: u32,
    /// The number of calls executed to attempt to break invariants in one run.
    ///
    /// When `min_depth` is set, this is the maximum depth and each run samples a depth in
    /// `[min_depth, depth]`.
    pub depth: u32,
    /// Optional minimum number of calls per invariant run.
    ///
    /// When set to a value in `1..depth`, each run samples a depth uniformly in
    /// `[min_depth, depth]` instead of always using `depth`. Invalid values (`0`, `>= depth`, or
    /// when `depth == 0`) fall back to the fixed-depth behavior.
    #[serde(default)]
    pub min_depth: Option<u32>,
    /// Worker selection mode used to shard invariant runs.
    ///
    /// Defaults to `1` for reproducible seeded campaigns. Use `auto` to derive the worker count
    /// from `--jobs`, or a positive integer for an explicit worker count.
    pub workers: InvariantWorkers,
    /// Fails the invariant fuzzing if a revert occurs
    pub fail_on_revert: bool,
    /// Allows overriding an unsafe external call when running invariant tests. eg. reentrancy
    /// checks
    pub call_override: bool,
    /// The fuzz dictionary configuration
    #[serde(flatten)]
    pub dictionary: FuzzDictionaryConfig,
    /// The maximum number of attempts to shrink the sequence
    pub shrink_run_limit: u32,
    /// The maximum number of rejects via `vm.assume` which can be encountered during a single
    /// invariant run.
    pub max_assume_rejects: u32,
    /// Number of runs to execute and include in the gas report.
    pub gas_report_samples: u32,
    /// The fuzz corpus configuration.
    #[serde(flatten)]
    pub corpus: FuzzCorpusConfig,
    /// Path where invariant failures are recorded and replayed.
    pub failure_persist_dir: Option<PathBuf>,
    /// Whether to collect and display fuzzed selectors metrics.
    pub show_metrics: bool,
    /// Optional campaign-global timeout (in seconds) for each invariant test.
    pub timeout: Option<u32>,
    /// Display counterexample as solidity calls.
    pub show_solidity: bool,
    /// Maximum time (in seconds) between generated txs.
    pub max_time_delay: Option<u32>,
    /// Maximum number of blocks elapsed between generated txs.
    pub max_block_delay: Option<u32>,
    /// Number of calls to execute between invariant assertions.
    ///
    /// - `0`: Only assert on the last call of each run (fastest, but may miss exact breaking call)
    /// - `1` (default): Assert after every call (current behavior, most precise)
    /// - `N`: Assert every N calls AND always on the last call
    ///
    /// Example: `check_interval = 10` means assert after calls 10, 20, 30, ... and the last call.
    pub check_interval: u32,
}

impl Default for InvariantConfig {
    fn default() -> Self {
        Self {
            runs: 256,
            depth: 500,
            // BENCH-ONLY: random depth enabled by default so `derek bench invariant` exercises it.
            // Revert to `None` before merging the opt-in PR.
            min_depth: Some(1),
            workers: InvariantWorkers::default(),
            fail_on_revert: false,
            call_override: false,
            dictionary: FuzzDictionaryConfig { dictionary_weight: 80, ..Default::default() },
            shrink_run_limit: 5000,
            max_assume_rejects: 65536,
            gas_report_samples: 256,
            corpus: FuzzCorpusConfig::default(),
            failure_persist_dir: None,
            show_metrics: true,
            timeout: None,
            show_solidity: false,
            max_time_delay: None,
            max_block_delay: None,
            check_interval: 1,
        }
    }
}

impl InvariantConfig {
    /// Creates invariant configuration to write failures in `{PROJECT_ROOT}/cache/fuzz` dir.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { failure_persist_dir: Some(cache_dir), ..Default::default() }
    }

    /// Returns true if generated invariant calls may advance block time or height.
    pub const fn has_delay(&self) -> bool {
        self.max_block_delay.is_some() || self.max_time_delay.is_some()
    }

    /// Returns the `[min_depth, depth]` range to sample per-run depths from, or `None` when random
    /// depth is disabled (no `min_depth`, `depth == 0`, or an invalid `min_depth`).
    ///
    /// A valid `min_depth` is in `1..depth`.
    pub const fn random_depth_range(&self) -> Option<(u32, u32)> {
        match self.min_depth {
            Some(min) if min >= 1 && self.depth > 0 && min < self.depth => Some((min, self.depth)),
            _ => None,
        }
    }

    /// Returns the estimated per-run depth used by campaign sizing heuristics.
    ///
    /// Uses the rounded-up average of the random depth range when enabled, otherwise `depth`.
    pub const fn estimated_depth(&self) -> u32 {
        match self.random_depth_range() {
            Some((min, max)) => (min + max).div_ceil(2),
            None => self.depth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_depth_range_is_only_active_for_valid_min_depth() {
        let cfg = |depth, min_depth| InvariantConfig { depth, min_depth, ..Default::default() };
        // Disabled by default.
        assert_eq!(cfg(500, None).random_depth_range(), None);
        // Valid range.
        assert_eq!(cfg(500, Some(10)).random_depth_range(), Some((10, 500)));
        assert_eq!(cfg(500, Some(1)).random_depth_range(), Some((1, 500)));
        // Invalid: zero min, min >= depth, depth zero.
        assert_eq!(cfg(500, Some(0)).random_depth_range(), None);
        assert_eq!(cfg(500, Some(500)).random_depth_range(), None);
        assert_eq!(cfg(500, Some(900)).random_depth_range(), None);
        assert_eq!(cfg(0, Some(1)).random_depth_range(), None);
    }

    #[test]
    fn estimated_depth_uses_rounded_average_when_random() {
        let cfg = |depth, min_depth| InvariantConfig { depth, min_depth, ..Default::default() };
        assert_eq!(cfg(500, None).estimated_depth(), 500);
        assert_eq!(cfg(500, Some(100)).estimated_depth(), 300);
        assert_eq!(cfg(10, Some(1)).estimated_depth(), 6);
        // Invalid min_depth falls back to fixed depth.
        assert_eq!(cfg(500, Some(0)).estimated_depth(), 500);
    }

    #[test]
    fn min_depth_defaults_to_none() {
        assert_eq!(InvariantConfig::default().min_depth, None);
        // `min_depth` deserializes via `#[serde(default)]` when omitted.
        let serialized = serde_json::to_value(InvariantConfig::default()).unwrap();
        assert_eq!(serialized["min_depth"], serde_json::Value::Null);
        let mut with_min = serialized;
        with_min["min_depth"] = serde_json::json!(25);
        let cfg: InvariantConfig = serde_json::from_value(with_min).unwrap();
        assert_eq!(cfg.min_depth, Some(25));
    }

    #[test]
    fn invariant_workers_accept_auto_and_fixed_counts() {
        assert_eq!("AUTO".parse::<InvariantWorkers>().unwrap(), InvariantWorkers::Auto);
        assert_eq!(
            serde_json::from_str::<InvariantWorkers>(r#""auto""#).unwrap(),
            InvariantWorkers::Auto
        );
        assert_eq!(
            serde_json::from_str::<InvariantWorkers>(r#"4"#).unwrap(),
            InvariantWorkers::Fixed(NonZeroUsize::new(4).unwrap())
        );
        assert_eq!(
            serde_json::from_str::<InvariantWorkers>(r#""4""#).unwrap(),
            InvariantWorkers::Fixed(NonZeroUsize::new(4).unwrap())
        );
    }

    #[test]
    fn invariant_workers_default_to_one() {
        assert_eq!(InvariantWorkers::default(), InvariantWorkers::Fixed(NonZeroUsize::MIN));
        assert_eq!(InvariantConfig::default().workers, InvariantWorkers::Fixed(NonZeroUsize::MIN));
    }

    #[test]
    fn invariant_workers_reject_zero() {
        let err = serde_json::from_str::<InvariantWorkers>(r#"0"#).unwrap_err();
        assert!(err.to_string().contains("greater than 0"));
    }
}
