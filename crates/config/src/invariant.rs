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

/// Campaign-local corpus synchronization mode for parallel invariant workers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvariantCorpusSyncMode {
    /// Keep the existing behavior: workers attempt to import sibling corpus candidates each run.
    #[default]
    Eager,
    /// Import sibling corpus candidates only after a worker stops finding new coverage.
    OnStall,
    /// Disable campaign-local corpus sharing while preserving final merged persistence.
    Off,
}

impl FromStr for InvariantCorpusSyncMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "eager" => Ok(Self::Eager),
            "on-stall" | "on_stall" | "onstall" => Ok(Self::OnStall),
            "off" | "none" | "disabled" => Ok(Self::Off),
            value => Err(format!(
                "invalid invariant corpus sync mode `{value}`; expected `eager`, `on-stall`, or `off`"
            )),
        }
    }
}

/// Controls campaign-local corpus exchange between parallel invariant workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantCorpusSyncConfig {
    /// Synchronization trigger used by parallel coverage-guided invariant workers.
    pub mode: InvariantCorpusSyncMode,
    /// Number of consecutive worker-local runs without new coverage before `on-stall` pulls.
    pub stall_runs: u32,
    /// Seconds without new coverage before `on-stall` pulls.
    pub stall_timeout: u64,
    /// Maximum number of sibling candidates replayed by a worker in one sync attempt.
    pub max_batch: usize,
}

impl Default for InvariantCorpusSyncConfig {
    fn default() -> Self {
        Self {
            mode: InvariantCorpusSyncMode::default(),
            stall_runs: 128,
            stall_timeout: 30,
            max_batch: 64,
        }
    }
}

/// Contains for invariant testing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantConfig {
    /// The number of runs that must execute for each invariant test group.
    pub runs: u32,
    /// The number of calls executed to attempt to break invariants in one run.
    pub depth: u32,
    /// Worker selection mode used to shard invariant runs.
    ///
    /// Defaults to `1` for reproducible seeded campaigns. Use `auto` to derive the worker count
    /// from `--jobs`, or a positive integer for an explicit worker count.
    pub workers: InvariantWorkers,
    /// Campaign-local corpus synchronization policy for parallel invariant workers.
    pub corpus_sync: InvariantCorpusSyncConfig,
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
            workers: InvariantWorkers::default(),
            corpus_sync: InvariantCorpusSyncConfig::default(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn invariant_corpus_sync_mode_accepts_aliases() {
        assert_eq!(
            "eager".parse::<InvariantCorpusSyncMode>().unwrap(),
            InvariantCorpusSyncMode::Eager
        );
        assert_eq!(
            "on_stall".parse::<InvariantCorpusSyncMode>().unwrap(),
            InvariantCorpusSyncMode::OnStall
        );
        assert_eq!(
            "none".parse::<InvariantCorpusSyncMode>().unwrap(),
            InvariantCorpusSyncMode::Off
        );
    }
}
