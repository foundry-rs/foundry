//! Formal symbolic benchmark suite definitions and result parsing.

use crate::results::SymbolicBenchmarkSummary;
use eyre::{Result, WrapErr};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

const FARCASTER_PATH: &str = "test/FarcasterNativeSymbolic.t.sol";
const FARCASTER_TEST: &str = include_str!("../fixtures/symbolic/FarcasterNativeSymbolic.t.sol");
const PREFIX: &str =
    "FOUNDRY_DYNAMIC_TEST_LINKING=false FOUNDRY_LINT_LINT_ON_BUILD=false FOUNDRY_ISOLATE=false";
const SOLADY_MATCH: &str = "check_(SaturatingAddEquivalence|SaturatingMulEquivalence|HasDuplicateHashmapCapacityTrickEquivalence|IsPermit2AndValueIsNotInfinityTrickEquivalence|IsNotUint256MaxTrickEquivalence|DelayRestriction|OperationStateDifferentialTrick|CarryBoundsTrick|SafeCastInt256ToIntTrickEquivalence|P256Normalized|AuxPackEquivalence|EcrecoverTrickEquivalence|EcrecoverLoopTrick)";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fixture {
    Solady,
    Angstrom,
    Farcaster,
    Generic,
}

impl Fixture {
    pub const fn identify(org: &str, repo: &str) -> Self {
        if org.eq_ignore_ascii_case("vectorized") && repo.eq_ignore_ascii_case("solady") {
            Self::Solady
        } else if org.eq_ignore_ascii_case("sorellalabs") && repo.eq_ignore_ascii_case("angstrom") {
            Self::Angstrom
        } else if org.eq_ignore_ascii_case("farcasterxyz") && repo.eq_ignore_ascii_case("contracts")
        {
            Self::Farcaster
        } else {
            Self::Generic
        }
    }

    pub fn build_command(self) -> String {
        format!(
            "{PREFIX} {}",
            match self {
                Self::Angstrom => "forge build --root contracts",
                _ => "forge build",
            }
        )
    }

    pub fn test_command(self) -> String {
        match self {
            Self::Solady => {
                format!("{PREFIX} forge test --symbolic --json --match-test '{SOLADY_MATCH}'")
            }
            Self::Angstrom => format!(
                "{PREFIX} forge test --root contracts --symbolic --json --symbolic-timeout 5 --match-path test/libraries/X128MathLib.t.sol --match-test check_matchesSolady_fullMulX128"
            ),
            Self::Farcaster => format!(
                "{PREFIX} forge test --symbolic --json --symbolic-timeout 5 --match-path '{FARCASTER_PATH}' --match-test 'check_'"
            ),
            Self::Generic => format!("{PREFIX} forge test --symbolic --json"),
        }
    }
}

/// Error-safe installation of the Farcaster benchmark overlay.
pub struct Overlay {
    path: Option<PathBuf>,
}

impl Overlay {
    pub fn install(root: &Path, fixture: Fixture) -> Result<Self> {
        if !matches!(fixture, Fixture::Farcaster) {
            return Ok(Self { path: None });
        }
        let path = root.join(FARCASTER_PATH);
        if path.exists() {
            eyre::bail!("refusing to overwrite existing {}", path.display());
        }
        if let Err(write_err) = fs::write(&path, FARCASTER_TEST) {
            if let Err(cleanup_err) = fs::remove_file(&path)
                && cleanup_err.kind() != std::io::ErrorKind::NotFound
            {
                eyre::bail!(
                    "failed to install Farcaster symbolic fixture: {write_err}; failed to remove partially written {}: {cleanup_err}",
                    path.display()
                );
            }
            return Err(write_err).wrap_err("failed to install Farcaster symbolic fixture");
        }
        Ok(Self { path: Some(path) })
    }

    pub fn finish(mut self) -> Result<()> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> Result<()> {
        if let Some(path) = &self.path {
            fs::remove_file(path).wrap_err_with(|| {
                format!("failed to remove symbolic fixture {}", path.display())
            })?;
            self.path = None;
        }
        Ok(())
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Metrics {
    pub paths: u64,
    pub solver_queries: u64,
    pub smt_queries: u64,
    pub sat_queries: u64,
    pub model_queries: u64,
    pub sat_cache_hits: u64,
    pub model_cache_hits: u64,
    pub heuristic_witnesses: u64,
    pub solver_time_ms: u64,
    pub smt_input_bytes: Option<u64>,
    pub smt_max_query_bytes: Option<u64>,
    pub smt_build_time_ms: Option<u64>,
    pub smt_max_query_time_ms: Option<u64>,
    #[serde(skip)]
    smt_input_bytes_compat: u64,
    #[serde(skip)]
    smt_max_query_bytes_compat: u64,
    #[serde(skip)]
    smt_build_time_ms_compat: u64,
    #[serde(skip)]
    smt_max_query_time_ms_compat: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Passed,
    Failed,
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TestOutcome {
    pub suite: String,
    pub signature: String,
    pub status: OutcomeStatus,
}

impl TestOutcome {
    fn identity(&self) -> String {
        format!("{}::{}", self.suite, self.signature)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ParsedRun {
    pub outcomes: Vec<TestOutcome>,
    pub passed: usize,
    pub failed: usize,
    pub incomplete: usize,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sample {
    pub wall_time_seconds: f64,
    pub exit_code: i32,
    #[serde(flatten)]
    pub run: ParsedRun,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sidecar {
    pub schema: &'static str,
    pub schema_version: u32,
    pub fixture: FixtureMetadata,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FixtureMetadata {
    pub name: Fixture,
    pub repository: String,
    pub revision: String,
    pub build_command: String,
    pub test_command: String,
}

impl Sidecar {
    pub fn new(
        fixture: Fixture,
        repository: &str,
        revision: &str,
        build_command: &str,
        test_command: &str,
        samples: Vec<Sample>,
    ) -> Self {
        Self {
            schema: "foundry:symbolic-benchmark@v1",
            schema_version: 1,
            fixture: FixtureMetadata {
                name: fixture,
                repository: repository.to_string(),
                revision: revision.to_string(),
                build_command: build_command.to_string(),
                test_command: test_command.to_string(),
            },
            samples,
        }
    }
}

pub fn parse(stdout: &[u8]) -> Result<ParsedRun> {
    let json: Value =
        serde_json::from_slice(stdout).wrap_err("invalid forge test --json output")?;
    let suites = json.as_object().ok_or_else(|| eyre::eyre!("expected JSON object"))?;
    let stable = suites
        .values()
        .filter_map(|s| s.get("test_results").and_then(Value::as_object))
        .flat_map(|r| r.values())
        .any(|r| r.get("symbolic").is_some());
    let mut run = ParsedRun {
        outcomes: Vec::new(),
        passed: 0,
        failed: 0,
        incomplete: 0,
        metrics: Metrics::default(),
    };
    for (suite_name, suite) in suites {
        let Some(results) = suite.get("test_results").and_then(Value::as_object) else { continue };
        for (identity, result) in results {
            let (stats, status) = if stable {
                let Some(symbolic) = result.get("symbolic") else { continue };
                if symbolic.get("schema_version").and_then(Value::as_u64) != Some(1) {
                    eyre::bail!("unknown or missing symbolic schema_version");
                }
                let status = symbolic
                    .get("status")
                    .and_then(Value::as_str)
                    .ok_or_else(|| eyre::eyre!("missing symbolic.status"))?;
                let status = match status {
                    "pass" => OutcomeStatus::Passed,
                    "fail_counterexample" => OutcomeStatus::Failed,
                    "incomplete" => OutcomeStatus::Incomplete,
                    _ => {
                        eyre::bail!("unknown symbolic.status {status}");
                    }
                };
                let stats = symbolic
                    .pointer("/solver/stats")
                    .filter(|stats| stats.is_object())
                    .ok_or_else(|| eyre::eyre!("missing or invalid symbolic.solver.stats"))?;
                (stats, status)
            } else {
                let Some(stats) = result.pointer("/kind/Symbolic") else { continue };
                let status = result.get("status").and_then(Value::as_str).unwrap_or_default();
                let reason = result.get("reason").and_then(Value::as_str).unwrap_or_default();
                let status = if status == "Success" {
                    OutcomeStatus::Passed
                } else if reason.contains("incomplete symbolic execution") {
                    OutcomeStatus::Incomplete
                } else {
                    OutcomeStatus::Failed
                };
                (stats, status)
            };
            run.outcomes.push(TestOutcome {
                suite: suite_name.clone(),
                signature: identity.clone(),
                status,
            });
            add_metrics(&mut run.metrics, stats, stable, run.outcomes.len() == 1)?;
        }
    }
    if run.outcomes.is_empty() {
        eyre::bail!("forge symbolic benchmark produced no symbolic test results");
    }
    run.outcomes.sort_by_key(TestOutcome::identity);
    run.passed =
        run.outcomes.iter().filter(|outcome| outcome.status == OutcomeStatus::Passed).count();
    run.failed =
        run.outcomes.iter().filter(|outcome| outcome.status == OutcomeStatus::Failed).count();
    run.incomplete =
        run.outcomes.iter().filter(|outcome| outcome.status == OutcomeStatus::Incomplete).count();
    Ok(run)
}

fn add_metrics(out: &mut Metrics, stats: &Value, stable: bool, first: bool) -> Result<()> {
    macro_rules! add {
        ($field:ident) => {
            out.$field += required(stats, stringify!($field), stable)?;
        };
    }
    add!(paths);
    add!(solver_queries);
    add!(smt_queries);
    add!(sat_queries);
    add!(model_queries);
    add!(sat_cache_hits);
    add!(model_cache_hits);
    add!(heuristic_witnesses);
    add!(solver_time_ms);
    optional_add(
        &mut out.smt_input_bytes,
        &mut out.smt_input_bytes_compat,
        stats,
        "smt_input_bytes",
        first,
    )?;
    optional_add(
        &mut out.smt_build_time_ms,
        &mut out.smt_build_time_ms_compat,
        stats,
        "smt_build_time_ms",
        first,
    )?;
    optional_max(
        &mut out.smt_max_query_bytes,
        &mut out.smt_max_query_bytes_compat,
        stats,
        "smt_max_query_bytes",
        first,
    )?;
    optional_max(
        &mut out.smt_max_query_time_ms,
        &mut out.smt_max_query_time_ms_compat,
        stats,
        "smt_max_query_time_ms",
        first,
    )?;
    Ok(())
}

fn required(v: &Value, key: &str, strict: bool) -> Result<u64> {
    match v.get(key) {
        Some(v) => v.as_u64().ok_or_else(|| eyre::eyre!("invalid metric {key}")),
        None if strict => Err(eyre::eyre!("missing metric {key}")),
        None => Ok(0),
    }
}

fn optional_add(
    out: &mut Option<u64>,
    compatibility: &mut u64,
    v: &Value,
    key: &str,
    first: bool,
) -> Result<()> {
    let value = v
        .get(key)
        .map(|n| n.as_u64().ok_or_else(|| eyre::eyre!("invalid metric {key}")))
        .transpose()?;
    *compatibility += value.unwrap_or_default();
    *out = if first { value } else { (*out).zip(value).map(|(old, value)| old + value) };
    Ok(())
}

fn optional_max(
    out: &mut Option<u64>,
    compatibility: &mut u64,
    v: &Value,
    key: &str,
    first: bool,
) -> Result<()> {
    let value = v
        .get(key)
        .map(|n| n.as_u64().ok_or_else(|| eyre::eyre!("invalid metric {key}")))
        .transpose()?;
    *compatibility = (*compatibility).max(value.unwrap_or_default());
    *out = if first { value } else { (*out).zip(value).map(|(old, value)| old.max(value)) };
    Ok(())
}

pub const fn compatibility(run: &ParsedRun) -> SymbolicBenchmarkSummary {
    let m = &run.metrics;
    SymbolicBenchmarkSummary {
        tests: run.outcomes.len(),
        passed: run.passed,
        failed: run.failed,
        incomplete: run.incomplete,
        paths: m.paths,
        solver_queries: m.solver_queries,
        smt_queries: m.smt_queries,
        sat_queries: m.sat_queries,
        model_queries: m.model_queries,
        sat_cache_hits: m.sat_cache_hits,
        model_cache_hits: m.model_cache_hits,
        heuristic_witnesses: m.heuristic_witnesses,
        solver_time_ms: m.solver_time_ms,
        smt_input_bytes: m.smt_input_bytes_compat,
        smt_max_query_bytes: m.smt_max_query_bytes_compat,
        smt_build_time_ms: m.smt_build_time_ms_compat,
        smt_max_query_time_ms: m.smt_max_query_time_ms_compat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(status: &str, schema: u64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"test/FarcasterNativeSymbolic.t.sol:FarcasterNativeSymbolicTest":{"test_results":{
            "check_migrateOnlyMigrator(uint24,address,address,address,uint40)":{"symbolic":{"schema_version":schema,"status":status,"bounds":{},"solver":{"name":"z3","command":null,"portfolio":[],"stats":{"paths":1,"solver_queries":2,"smt_queries":3,"sat_queries":4,"model_queries":5,"sat_cache_hits":6,"model_cache_hits":7,"heuristic_witnesses":8,"solver_time_ms":9,"smt_input_bytes":10,"smt_max_query_bytes":11,"smt_build_time_ms":12,"smt_max_query_time_ms":13}}}},
            "check_setMigratorOnlyOwner(uint24,address,address,address,address)":{"symbolic":{"schema_version":schema,"status":"incomplete","bounds":{},"solver":{"name":"z3","command":null,"portfolio":[],"stats":{"paths":10,"solver_queries":20,"smt_queries":30,"sat_queries":40,"model_queries":50,"sat_cache_hits":60,"model_cache_hits":70,"heuristic_witnesses":80,"solver_time_ms":90}}}}
        }}})).unwrap()
    }

    #[test]
    fn stable_parser_validates_statuses_and_missing_optional_metrics() {
        let run = parse(&stable("pass", 1)).unwrap();
        assert_eq!((run.passed, run.failed, run.incomplete), (1, 0, 1));
        assert_eq!(run.metrics.paths, 11);
        assert_eq!(run.metrics.solver_time_ms, 99);
        assert_eq!(run.metrics.smt_max_query_bytes, None);
        assert_eq!(run.metrics.smt_input_bytes, None);
        assert_eq!(run.metrics.smt_build_time_ms, None);
        assert_eq!(run.metrics.smt_max_query_time_ms, None);
        assert_eq!(
            run.outcomes[0].suite,
            "test/FarcasterNativeSymbolic.t.sol:FarcasterNativeSymbolicTest"
        );
        let compatibility = compatibility(&run);
        assert_eq!(compatibility.tests, 2);
        assert_eq!(compatibility.smt_input_bytes, 10);
    }

    #[test]
    fn rejects_unknown_stable_schema() {
        assert!(parse(&stable("pass", 2)).is_err());
    }

    #[test]
    fn aggregates_optional_metrics_only_when_every_test_reports_them() {
        let first = serde_json::json!({
            "paths": 1, "solver_queries": 1, "smt_queries": 1, "sat_queries": 1,
            "model_queries": 1, "sat_cache_hits": 1, "model_cache_hits": 1,
            "heuristic_witnesses": 1, "solver_time_ms": 1, "smt_input_bytes": 10,
            "smt_max_query_bytes": 8, "smt_build_time_ms": 2, "smt_max_query_time_ms": 4
        });
        let second = serde_json::json!({
            "paths": 1, "solver_queries": 1, "smt_queries": 1, "sat_queries": 1,
            "model_queries": 1, "sat_cache_hits": 1, "model_cache_hits": 1,
            "heuristic_witnesses": 1, "solver_time_ms": 1, "smt_input_bytes": 20,
            "smt_max_query_bytes": 7, "smt_build_time_ms": 3, "smt_max_query_time_ms": 9
        });
        let mut metrics = Metrics::default();
        add_metrics(&mut metrics, &first, true, true).unwrap();
        add_metrics(&mut metrics, &second, true, false).unwrap();
        assert_eq!(metrics.smt_input_bytes, Some(30));
        assert_eq!(metrics.smt_build_time_ms, Some(5));
        assert_eq!(metrics.smt_max_query_bytes, Some(8));
        assert_eq!(metrics.smt_max_query_time_ms, Some(9));
    }

    #[test]
    fn parses_whole_output_legacy_results() {
        let stats = serde_json::json!({"paths": 1, "solver_queries": 2});
        let stdout = serde_json::to_vec(&serde_json::json!({"test/FarcasterNativeSymbolic.t.sol:FarcasterNativeSymbolicTest":{"test_results":{
            "check_migrateOnlyMigrator(uint24,address,address,address,uint40)":{
                "kind":{"Symbolic":stats},"status":"Success"
            },
            "check_setMigratorOnlyOwner(uint24,address,address,address,address)":{
                "kind":{"Symbolic":stats},"status":"Failure",
                "reason":"incomplete symbolic execution (Timeout)"
            }
        }}}))
        .unwrap();
        let run = parse(&stdout).unwrap();
        assert_eq!((run.passed, run.failed, run.incomplete), (1, 0, 1));
        assert_eq!(run.metrics.paths, 2);
    }

    #[test]
    fn unknown_repository_uses_generic_symbolic_command() {
        let fixture = Fixture::identify("example", "contracts");
        assert!(matches!(fixture, Fixture::Generic));
        assert_eq!(fixture.test_command(), format!("{PREFIX} forge test --symbolic --json"));
    }

    #[test]
    fn metrics_serialize_the_complete_v1_counter_set() {
        let value = serde_json::to_value(Metrics::default()).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from([
                "heuristic_witnesses",
                "model_cache_hits",
                "model_queries",
                "paths",
                "sat_cache_hits",
                "sat_queries",
                "smt_build_time_ms",
                "smt_input_bytes",
                "smt_max_query_bytes",
                "smt_max_query_time_ms",
                "smt_queries",
                "solver_queries",
                "solver_time_ms",
            ])
        );
    }

    #[test]
    fn sidecar_retains_samples_and_revision() {
        let first = parse(&stable("pass", 1)).unwrap();
        let second = parse(&stable("fail_counterexample", 1)).unwrap();
        let samples = [first, second]
            .into_iter()
            .map(|run| Sample { wall_time_seconds: 1.0, exit_code: 0, run })
            .collect();
        let sidecar = Sidecar::new(
            Fixture::Farcaster,
            "farcasterxyz/contracts",
            "test-revision",
            "forge build",
            "forge test --symbolic --json",
            samples,
        );
        assert_eq!(sidecar.fixture.revision, "test-revision");
        assert_eq!(sidecar.samples.len(), 2);
    }

    #[test]
    fn overlay_is_explicitly_removed_and_existing_file_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("test")).unwrap();
        let path = dir.path().join(FARCASTER_PATH);
        let overlay = Overlay::install(dir.path(), Fixture::Farcaster).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), FARCASTER_TEST);
        overlay.finish().unwrap();
        assert!(!path.exists());
        fs::write(&path, "unexpected").unwrap();
        assert!(Overlay::install(dir.path(), Fixture::Farcaster).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "unexpected");
    }
}
