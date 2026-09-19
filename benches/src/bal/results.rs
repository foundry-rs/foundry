//! Recompute auditable summaries without discarding unsuccessful scheduled attempts.

use super::{
    ActualPath, Arm,
    proxy::{Completion, RpcEvent, Snapshot, is_bal_method},
    read_json, write_json,
};
use eyre::{Context, Result, ensure};
use foundry_bench::results::{CommonBenchmark, CommonBenchmarkResult, Metric, RunnerMetadata};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    fs,
    path::Path,
};

/// One scheduled child, including unsuccessful and censored attempts.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sample {
    pub schema_version: u8,
    pub id: String,
    pub case_id: String,
    pub phase: String,
    pub round: usize,
    pub order: usize,
    pub arm: Arm,
    pub actual_path: ActualPath,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub wall_time_seconds: Option<f64>,
    pub observed_duration_seconds: f64,
    pub stdout_sha256: String,
    pub local_gas: Option<u64>,
    pub execution_success: Option<bool>,
    pub correctness: String,
    pub fault: Option<String>,
    pub synthetic: bool,
    pub rpc_at_exit: Snapshot,
    pub rpc: Snapshot,
    pub bal_events: Vec<RpcEvent>,
}

impl Sample {
    fn eligible(&self) -> bool {
        !self.timed_out
            && self.exit_code == Some(0)
            && self.wall_time_seconds.is_some()
            && self.correctness == "equivalent"
            && matches!(
                self.actual_path,
                ActualPath::BalHit | ActualPath::ReplayAfterProbe | ActualPath::ReplayNoProbe
            )
    }
}

/// Quantiles use linear interpolation between sorted observations.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Distribution {
    count: usize,
    median: f64,
    q1: f64,
    q3: f64,
    iqr: f64,
    min: f64,
    max: f64,
}

fn distribution(values: impl IntoIterator<Item = f64>) -> Option<Distribution> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let quantile = |fraction: f64| {
        let position = fraction * (values.len() - 1) as f64;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        (values[upper] - values[lower]).mul_add(position.fract(), values[lower])
    };
    let q1 = quantile(0.25);
    let q3 = quantile(0.75);
    Some(Distribution {
        count: values.len(),
        median: quantile(0.5),
        q1,
        q3,
        iqr: q3 - q1,
        min: values[0],
        max: values[values.len() - 1],
    })
}

#[derive(Debug, Serialize)]
struct Group {
    case_id: String,
    arm: Arm,
    fault: Option<String>,
    synthetic: bool,
    conditional_on_bal_hit: bool,
    attempts: usize,
    completed: usize,
    eligible: usize,
    failed: usize,
    unknown: usize,
    censored: usize,
    correctness_blocked: usize,
    actual_paths: BTreeMap<String, usize>,
    completed_wall_seconds: Option<Distribution>,
    eligible_wall_seconds: Option<Distribution>,
    rpc_at_exit: RpcTotals,
    rpc_after_cleanup: RpcTotals,
    rpc_per_attempt: BTreeMap<String, Distribution>,
    bal_http_elapsed_seconds: Option<Distribution>,
    bal_upstream_elapsed_seconds: Option<Distribution>,
    bal_exchanges: usize,
    bal_incomplete_exchanges: usize,
    bal_mixed_batch_exchanges: usize,
    bal_exchange_observed_body_bytes: u64,
    bal_exchange_cleanup_body_bytes: u64,
    bal_result_json_bytes: u64,
    bal_results_with_known_size: usize,
}

/// Aggregate counts omit the per-snapshot response analysis state.
#[derive(Debug, Default, Serialize)]
struct RpcTotals {
    client_requests_by_method: BTreeMap<String, u64>,
    upstream_requests_by_method: BTreeMap<String, u64>,
    client_http_exchanges: u64,
    upstream_http_exchanges: u64,
    injected_responses: u64,
    client_response_body_bytes: u64,
    upstream_response_body_bytes: u64,
    repeated_requests: u64,
    errors: u64,
    active_http_exchanges: u64,
}

fn sum_rpc<'a>(snapshots: impl IntoIterator<Item = &'a Snapshot>) -> RpcTotals {
    let mut totals = RpcTotals::default();
    for snapshot in snapshots {
        for (counts, total_counts) in [
            (&snapshot.client_requests_by_method, &mut totals.client_requests_by_method),
            (&snapshot.upstream_requests_by_method, &mut totals.upstream_requests_by_method),
        ] {
            for (method, count) in counts {
                *total_counts.entry(method.clone()).or_default() += count;
            }
        }
        totals.client_http_exchanges += snapshot.client_http_exchanges;
        totals.upstream_http_exchanges += snapshot.upstream_http_exchanges;
        totals.injected_responses += snapshot.injected_responses;
        totals.client_response_body_bytes += snapshot.client_response_body_bytes;
        totals.upstream_response_body_bytes += snapshot.upstream_response_body_bytes;
        totals.repeated_requests += snapshot.repeated_requests;
        totals.errors += snapshot.errors;
        totals.active_http_exchanges += snapshot.active_http_exchanges;
    }
    totals
}

fn summarize(samples: &[&Sample], conditional: bool) -> Group {
    let first = samples[0];
    let eligible = samples.iter().copied().filter(|sample| sample.eligible()).collect::<Vec<_>>();
    let mut actual_paths = BTreeMap::new();
    let mut bal_http_elapsed = Vec::new();
    let mut bal_upstream_elapsed = Vec::new();
    let mut group = Group {
        case_id: first.case_id.clone(),
        arm: first.arm,
        fault: first.fault.clone(),
        synthetic: first.synthetic,
        conditional_on_bal_hit: conditional,
        attempts: samples.len(),
        completed: samples
            .iter()
            .filter(|sample| {
                !sample.timed_out
                    && sample.exit_code.is_some()
                    && sample.wall_time_seconds.is_some()
            })
            .count(),
        eligible: eligible.len(),
        failed: samples
            .iter()
            .filter(|sample| {
                sample.actual_path == ActualPath::Failed || sample.exit_code != Some(0)
            })
            .count(),
        unknown: samples.iter().filter(|sample| sample.actual_path == ActualPath::Unknown).count(),
        censored: samples.iter().filter(|sample| sample.timed_out).count(),
        correctness_blocked: samples
            .iter()
            .filter(|sample| sample.correctness == "correctness_blocked")
            .count(),
        actual_paths: BTreeMap::new(),
        completed_wall_seconds: distribution(
            samples
                .iter()
                .filter(|sample| !sample.timed_out)
                .filter_map(|sample| sample.wall_time_seconds),
        ),
        eligible_wall_seconds: distribution(
            eligible.iter().filter_map(|sample| sample.wall_time_seconds),
        ),
        rpc_at_exit: sum_rpc(samples.iter().map(|sample| &sample.rpc_at_exit)),
        rpc_after_cleanup: sum_rpc(samples.iter().map(|sample| &sample.rpc)),
        rpc_per_attempt: metric_distributions(
            &samples.iter().map(|sample| rpc_metrics(sample)).collect::<Vec<_>>(),
        ),
        bal_http_elapsed_seconds: None,
        bal_upstream_elapsed_seconds: None,
        bal_exchanges: 0,
        bal_incomplete_exchanges: 0,
        bal_mixed_batch_exchanges: 0,
        bal_exchange_observed_body_bytes: 0,
        bal_exchange_cleanup_body_bytes: 0,
        bal_result_json_bytes: 0,
        bal_results_with_known_size: 0,
    };
    for sample in samples {
        let path = match sample.actual_path {
            ActualPath::BalHit => "bal_hit",
            ActualPath::ReplayAfterProbe => "replay_after_probe",
            ActualPath::ReplayNoProbe => "replay_no_probe",
            ActualPath::Failed => "failed",
            ActualPath::Unknown => "unknown",
        };
        *actual_paths.entry(path.to_owned()).or_default() += 1;
        for event in &sample.bal_events {
            group.bal_exchanges += 1;
            let response = &event.response;
            group.bal_exchange_observed_body_bytes += response.body_bytes;
            group.bal_exchange_cleanup_body_bytes += response.cleanup_body_bytes;
            if response.completion == Completion::Eof {
                if let Some(elapsed) = response.elapsed_seconds {
                    bal_http_elapsed.push(elapsed);
                }
            } else {
                group.bal_incomplete_exchanges += 1;
            }
            if let Some(upstream) = &event.upstream
                && upstream.completion == Completion::Eof
                && let Some(elapsed) = upstream.elapsed_seconds
            {
                bal_upstream_elapsed.push(elapsed);
            }
            group.bal_mixed_batch_exchanges +=
                usize::from(event.calls.iter().any(|call| !is_bal_method(&call.method)));
            for call in &event.calls {
                if is_bal_method(&call.method)
                    && let Some(bytes) = call.result_json_bytes
                {
                    group.bal_result_json_bytes += bytes;
                    group.bal_results_with_known_size += 1;
                }
            }
        }
    }
    group.actual_paths = actual_paths;
    group.bal_http_elapsed_seconds = distribution(bal_http_elapsed);
    group.bal_upstream_elapsed_seconds = distribution(bal_upstream_elapsed);
    group
}

fn group_samples(samples: &[Sample]) -> Vec<Group> {
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for sample in samples.iter().filter(|sample| sample.phase == "measured") {
        groups
            .entry((&sample.case_id, sample.arm, &sample.fault, sample.synthetic))
            .or_default()
            .push(sample);
    }
    let mut summaries = Vec::new();
    for samples in groups.into_values() {
        summaries.push(summarize(&samples, false));
        if samples[0].arm == Arm::Auto {
            let hits = samples
                .iter()
                .copied()
                .filter(|sample| sample.actual_path == ActualPath::BalHit)
                .collect::<Vec<_>>();
            if !hits.is_empty() {
                summaries.push(summarize(&hits, true));
            }
        }
    }
    summaries
}

/// RPC costs at process exit; BAL body bytes include the observed cleanup tail separately.
fn rpc_metrics(sample: &Sample) -> BTreeMap<String, f64> {
    let mut metrics = BTreeMap::new();
    for (name, value) in [
        ("client_requests", sample.rpc_at_exit.client_requests_by_method.values().sum()),
        ("upstream_requests", sample.rpc_at_exit.upstream_requests_by_method.values().sum()),
        ("client_response_body_bytes", sample.rpc_at_exit.client_response_body_bytes),
        ("upstream_response_body_bytes", sample.rpc_at_exit.upstream_response_body_bytes),
        (
            "bal_observed_body_bytes",
            sample.bal_events.iter().map(|event| event.response.body_bytes).sum(),
        ),
        (
            "bal_cleanup_body_bytes",
            sample.bal_events.iter().map(|event| event.response.cleanup_body_bytes).sum(),
        ),
    ] {
        metrics.insert(name.to_owned(), value as f64);
    }
    for (direction, counts) in [
        ("client_requests_by_method", &sample.rpc_at_exit.client_requests_by_method),
        ("upstream_requests_by_method", &sample.rpc_at_exit.upstream_requests_by_method),
    ] {
        for (method, count) in counts {
            metrics.insert(format!("{direction}.{method}"), *count as f64);
        }
    }
    metrics
}

fn metric_distributions(values: &[BTreeMap<String, f64>]) -> BTreeMap<String, Distribution> {
    values
        .iter()
        .flat_map(|value| value.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|name| {
            distribution(values.iter().map(|value| value.get(name).copied().unwrap_or(0.0)))
                .map(|distribution| (name.clone(), distribution))
        })
        .collect()
}

/// Pair by case and round, retaining missing, duplicate and invalid scheduled attempts.
#[derive(Debug, Serialize)]
struct Comparison {
    case_id: String,
    synthetic: bool,
    fault: Option<String>,
    scheduled_pairs: usize,
    observed_auto_attempts: usize,
    observed_replay_attempts: usize,
    valid_pairs: usize,
    missing_pairs: usize,
    duplicate_pairs: usize,
    invalid_pairs: usize,
    unexpected_rounds: usize,
    complete: bool,
    auto_wall_seconds: Option<Distribution>,
    replay_wall_seconds: Option<Distribution>,
    paired_wall_delta_seconds: Option<Distribution>,
    speedup: Option<f64>,
    rpc_delta_per_pair: Option<BTreeMap<String, Distribution>>,
}

fn compare_samples(samples: &[Sample], manifest: &Value) -> Vec<Comparison> {
    let mut cases = BTreeMap::<_, Vec<_>>::new();
    if let Some(panel) = manifest["panel"]["cases"].as_array() {
        for case in panel {
            if let Some(id) = case["id"].as_str() {
                cases.entry(id).or_default();
            }
        }
    }
    for sample in samples.iter().filter(|sample| {
        sample.phase == "measured" && matches!(sample.arm, Arm::Auto | Arm::Replay)
    }) {
        cases.entry(&sample.case_id).or_default().push(sample);
    }
    let scheduled = if manifest["warmup_only"] == true {
        Some(BTreeSet::new())
    } else if let Some(rounds) = manifest["rounds"].as_u64() {
        let offset = manifest["round_offset"].as_u64().unwrap_or(0);
        Some((offset..offset.saturating_add(rounds)).map(|round| round as usize).collect())
    } else {
        None
    };
    cases
        .into_iter()
        .map(|(case_id, samples)| {
            let observed = samples.iter().map(|sample| sample.round).collect::<BTreeSet<_>>();
            let rounds = scheduled.as_ref().unwrap_or(&observed);
            let mut comparison = Comparison {
                case_id: case_id.to_owned(),
                synthetic: samples.first().is_some_and(|sample| sample.synthetic),
                fault: samples.first().and_then(|sample| sample.fault.clone()),
                scheduled_pairs: rounds.len(),
                observed_auto_attempts: samples
                    .iter()
                    .filter(|sample| sample.arm == Arm::Auto)
                    .count(),
                observed_replay_attempts: samples
                    .iter()
                    .filter(|sample| sample.arm == Arm::Replay)
                    .count(),
                valid_pairs: 0,
                missing_pairs: 0,
                duplicate_pairs: 0,
                invalid_pairs: 0,
                unexpected_rounds: observed.difference(rounds).count(),
                complete: false,
                auto_wall_seconds: None,
                replay_wall_seconds: None,
                paired_wall_delta_seconds: None,
                speedup: None,
                rpc_delta_per_pair: None,
            };
            let mut pairs = Vec::new();
            for round in rounds {
                let auto = samples
                    .iter()
                    .copied()
                    .filter(|sample| sample.round == *round && sample.arm == Arm::Auto)
                    .collect::<Vec<_>>();
                let replay = samples
                    .iter()
                    .copied()
                    .filter(|sample| sample.round == *round && sample.arm == Arm::Replay)
                    .collect::<Vec<_>>();
                comparison.missing_pairs += usize::from(auto.is_empty() || replay.is_empty());
                comparison.duplicate_pairs += usize::from(auto.len() > 1 || replay.len() > 1);
                if let ([auto], [replay]) = (auto.as_slice(), replay.as_slice()) {
                    if auto.eligible()
                        && replay.eligible()
                        && replay.actual_path == ActualPath::ReplayNoProbe
                        && auto.stdout_sha256 == replay.stdout_sha256
                        && auto.local_gas == replay.local_gas
                        && auto.execution_success == replay.execution_success
                    {
                        pairs.push((*auto, *replay));
                    } else {
                        comparison.invalid_pairs += 1;
                    }
                }
            }
            comparison.valid_pairs = pairs.len();
            comparison.complete = scheduled.as_ref().is_some_and(|rounds| !rounds.is_empty())
                && pairs.len() == rounds.len()
                && comparison.unexpected_rounds == 0;
            if comparison.complete {
                comparison.auto_wall_seconds =
                    distribution(pairs.iter().filter_map(|(auto, _)| auto.wall_time_seconds));
                comparison.replay_wall_seconds =
                    distribution(pairs.iter().filter_map(|(_, replay)| replay.wall_time_seconds));
                comparison.paired_wall_delta_seconds =
                    distribution(pairs.iter().map(|(auto, replay)| {
                        auto.wall_time_seconds.unwrap() - replay.wall_time_seconds.unwrap()
                    }));
                if let (Some(auto), Some(replay)) =
                    (&comparison.auto_wall_seconds, &comparison.replay_wall_seconds)
                    && auto.median > 0.0
                {
                    comparison.speedup = Some(replay.median / auto.median);
                }
                let deltas = pairs
                    .iter()
                    .map(|(auto, replay)| {
                        let mut delta = rpc_metrics(auto);
                        for (name, value) in rpc_metrics(replay) {
                            *delta.entry(name).or_default() -= value;
                        }
                        delta
                    })
                    .collect::<Vec<_>>();
                comparison.rpc_delta_per_pair = Some(metric_distributions(&deltas));
            }
            comparison
        })
        .collect()
}

fn common_projection(
    groups: &[Group],
    commit: &str,
    runner: RunnerMetadata,
) -> Option<CommonBenchmarkResult> {
    let benchmarks = groups
        .iter()
        .filter_map(|group| {
            if group.conditional_on_bal_hit || group.fault.is_some() {
                return None;
            }
            let wall = group.eligible_wall_seconds.as_ref()?;
            let mut counters = BTreeMap::new();
            for (name, count) in [
                ("scheduled_attempts", group.attempts),
                ("valid_completed", group.eligible),
                ("failed", group.failed),
                ("unknown", group.unknown),
                ("censored", group.censored),
                ("correctness_blocked", group.correctness_blocked),
            ] {
                counters.insert(
                    name.to_owned(),
                    Metric { value: count as f64, unit: "count", statistic: "total" },
                );
            }
            for (path, count) in &group.actual_paths {
                counters.insert(
                    path.clone(),
                    Metric { value: *count as f64, unit: "count", statistic: "total" },
                );
            }
            Some(CommonBenchmark {
                name: format!(
                    "cast_run_bal/{}/{}/completed_valid{}",
                    group.case_id,
                    group.arm.name(),
                    if group.synthetic { "/synthetic" } else { "" }
                ),
                wall_time: Metric { value: wall.median, unit: "second", statistic: "median" },
                counters,
                solver: BTreeMap::new(),
            })
        })
        .collect::<Vec<_>>();
    (!benchmarks.is_empty()).then(|| CommonBenchmarkResult {
        schema_version: 1,
        repo: "foundry-rs/foundry".into(),
        commit: commit.into(),
        pr: None,
        runner,
        benchmarks,
    })
}

fn cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn timing(distribution: Option<&Distribution>) -> String {
    distribution.map_or_else(
        || "—".into(),
        |d| format!("{:.6} / {:.6} / {:.6} / {:.6}", d.median, d.iqr, d.min, d.max),
    )
}

fn median(distribution: Option<&Distribution>) -> String {
    distribution.map_or_else(|| "—".into(), |distribution| format!("{:.6}", distribution.median))
}

fn rpc_median(metrics: &BTreeMap<String, Distribution>, name: &str) -> String {
    metrics
        .get(name)
        .map_or_else(|| "—".into(), |distribution| format!("{:.1}", distribution.median))
}

/// Regenerate human and common-schema reports from the retained raw samples.
pub fn report(output_dir: &Path) -> Result<()> {
    let manifest_path = output_dir.join("manifest.json");
    let manifest: Value = read_json(&manifest_path)?;
    let runner = manifest
        .get("runner")
        .map(|runner| serde_json::from_value::<RunnerMetadata>(runner.clone()))
        .transpose()
        .wrap_err("invalid measurement runner metadata in run manifest")?;
    if let Some(runner) = &runner {
        ensure!(
            !runner.os.is_empty()
                && !runner.arch.is_empty()
                && runner.logical_cpus > 0
                && runner.image.as_ref().is_none_or(|image| !image.is_empty()),
            "invalid measurement runner metadata in run manifest"
        );
    }
    let has_runner = runner.is_some();
    ensure!(
        manifest["build"].get("auto").is_none() && manifest["build"].get("replay").is_none(),
        "legacy split-binary runs cannot produce same-binary BAL reports; rerun with one Cast binary"
    );
    let binary = manifest.get("binary").unwrap_or(&manifest["build"]["cast"]);
    ensure!(
        binary["sha256"].as_str().is_some_and(|hash| {
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        }),
        "run manifest is missing the single Cast binary SHA-256 identity"
    );
    if let Some(build_binary) = manifest["build"].get("cast") {
        ensure!(
            binary["sha256"] == build_binary["sha256"],
            "Cast binary identity differs from build provenance"
        );
    }
    let samples_path = output_dir.join("samples.jsonl");
    let mut samples = Vec::new();
    if samples_path.exists() {
        for line in fs::read_to_string(samples_path)?.lines().filter(|line| !line.trim().is_empty())
        {
            let sample: Sample = serde_json::from_str(line)?;
            ensure!(sample.schema_version == 1, "unsupported sample schema");
            ensure!(
                sample.observed_duration_seconds.is_finite()
                    && sample.observed_duration_seconds >= 0.0
                    && sample
                        .wall_time_seconds
                        .is_none_or(|value| value.is_finite() && value >= 0.0)
                    && (!sample.timed_out || sample.wall_time_seconds.is_none()),
                "invalid sample duration"
            );
            samples.push(sample);
        }
    }
    let groups = group_samples(&samples);
    let common = runner.and_then(|runner| {
        common_projection(
            &groups,
            manifest["build"]["source_sha"].as_str().unwrap_or("unknown"),
            runner,
        )
    });
    let has_common_projection = common.is_some();
    let comparisons = compare_samples(&samples, &manifest);
    write_json(
        &output_dir.join("summary.json"),
        &json!({
            "schema_version": 1, "groups": groups, "no_common_projection": !has_common_projection,
            "comparisons": comparisons, "quantiles": "linear_interpolation", "warmup_and_validation_excluded": true,
        }),
    )?;
    let common_path = output_dir.join("common-results.json");
    if let Some(common) = common {
        write_json(&common_path, &common)?;
    } else if common_path.exists() {
        fs::remove_file(common_path)?;
    }
    let mut report = String::from(
        "# Cast BAL benchmark\n\nAuto and replay use the same Cast binary; replay adds `--no-bal`. Comparisons pair measured rounds and require every scheduled pair to complete with equivalent output, gas and status. Missing, duplicated, failed, timed-out or mismatched attempts suppress the entire case's comparison. Delta is auto minus replay; speedup is replay median divided by auto median. No overall speedup is inferred from conditional BAL hits.\n\n",
    );
    report.push_str("| Case | Valid / scheduled pairs | Auto / replay median seconds | Paired delta median seconds | Speedup | Missing / duplicate / invalid pairs |\n| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for comparison in &comparisons {
        writeln!(
            report,
            "| {}{} | {} / {} | {} / {} | {} | {} | {} / {} / {} |",
            cell(&comparison.case_id),
            if comparison.synthetic { " (synthetic)" } else { "" },
            comparison.valid_pairs,
            comparison.scheduled_pairs,
            median(comparison.auto_wall_seconds.as_ref()),
            median(comparison.replay_wall_seconds.as_ref()),
            median(comparison.paired_wall_delta_seconds.as_ref()),
            comparison.speedup.map_or_else(|| "—".into(), |speedup| format!("{speedup:.3}×")),
            comparison.missing_pairs,
            comparison.duplicate_pairs,
            comparison.invalid_pairs,
        )?;
    }
    report.push_str("\nAll attempts remain in the diagnostics below. Wall distributions include completed failures; timeouts are censored. Common JSON contains valid completed equivalents with full attempt counters. The optional miss arm injects local method-not-found, excluding a real unsupported provider's round trip.\n\n");
    report.push_str("| Case | Arm | Attempts / complete / valid | Hit / fallback / no probe | Failed / unknown / censored / blocked | Wall seconds: median / IQR / min / max |\n| --- | --- | ---: | ---: | ---: | --- |\n");
    for group in groups.iter().filter(|group| group.fault.is_none()) {
        let name = format!(
            "{}{}{}",
            group.arm.name(),
            if group.conditional_on_bal_hit { " (given hit)" } else { "" },
            if group.synthetic { " (synthetic)" } else { "" }
        );
        writeln!(
            report,
            "| {} | {} | {} / {} / {} | {} / {} / {} | {} / {} / {} / {} | {} |",
            cell(&group.case_id),
            name,
            group.attempts,
            group.completed,
            group.eligible,
            group.actual_paths.get("bal_hit").unwrap_or(&0),
            group.actual_paths.get("replay_after_probe").unwrap_or(&0),
            group.actual_paths.get("replay_no_probe").unwrap_or(&0),
            group.failed,
            group.unknown,
            group.censored,
            group.correctness_blocked,
            timing(group.completed_wall_seconds.as_ref())
        )?;
    }
    if groups.iter().any(|group| group.fault.is_some()) {
        report.push_str("\n| Fault case | Fault | Arm | Attempts / complete / equivalent | Failed / unknown / censored / blocked | Wall seconds: median / IQR / min / max | Client / upstream RPCs | Observed paths |\n| --- | --- | --- | ---: | ---: | --- | ---: | --- |\n");
    }
    for group in
        groups.iter().filter(|group| group.fault.is_some() && !group.conditional_on_bal_hit)
    {
        writeln!(
            report,
            "| {} | {} | {} | {} / {} / {} | {} / {} / {} / {} | {} | {} / {} | {} |",
            cell(&group.case_id),
            cell(group.fault.as_deref().unwrap_or_default()),
            group.arm.name(),
            group.attempts,
            group.completed,
            group.eligible,
            group.failed,
            group.unknown,
            group.censored,
            group.correctness_blocked,
            timing(group.completed_wall_seconds.as_ref()),
            group.rpc_at_exit.client_requests_by_method.values().sum::<u64>(),
            group.rpc_at_exit.upstream_requests_by_method.values().sum::<u64>(),
            cell(&serde_json::to_string(&group.actual_paths)?)
        )?;
    }
    report.push_str("\n| Case / arm | Client / upstream RPCs | BAL exchanges / incomplete / mixed batch | BAL exchange observed bytes / cleanup bytes | BAL EOF seconds: median / IQR / min / max |\n| --- | ---: | ---: | ---: | --- |\n");
    for group in groups.iter().filter(|group| !group.conditional_on_bal_hit) {
        writeln!(
            report,
            "| {} / {} | {} / {} | {} / {} / {} | {} / {} | {} |",
            cell(&group.case_id),
            group.arm.name(),
            group.rpc_at_exit.client_requests_by_method.values().sum::<u64>(),
            group.rpc_at_exit.upstream_requests_by_method.values().sum::<u64>(),
            group.bal_exchanges,
            group.bal_incomplete_exchanges,
            group.bal_mixed_batch_exchanges,
            group.bal_exchange_observed_body_bytes,
            group.bal_exchange_cleanup_body_bytes,
            timing(group.bal_http_elapsed_seconds.as_ref())
        )?;
    }
    report.push_str("\nRPC medians below use every measured attempt, including failures and timeouts. Paired RPC deltas use the same complete cases as the elapsed-time comparison.\n\n| Case / arm | Client / upstream requests per attempt | Client / upstream response bytes per attempt | BAL observed / cleanup bytes per attempt |\n| --- | ---: | ---: | ---: |\n");
    for group in groups.iter().filter(|group| !group.conditional_on_bal_hit) {
        writeln!(
            report,
            "| {} / {} | {} / {} | {} / {} | {} / {} |",
            cell(&group.case_id),
            group.arm.name(),
            rpc_median(&group.rpc_per_attempt, "client_requests"),
            rpc_median(&group.rpc_per_attempt, "upstream_requests"),
            rpc_median(&group.rpc_per_attempt, "client_response_body_bytes"),
            rpc_median(&group.rpc_per_attempt, "upstream_response_body_bytes"),
            rpc_median(&group.rpc_per_attempt, "bal_observed_body_bytes"),
            rpc_median(&group.rpc_per_attempt, "bal_cleanup_body_bytes"),
        )?;
    }
    report.push_str("\n| Case | Paired median client / upstream request delta | Paired median client response byte delta | Paired median BAL observed byte delta |\n| --- | ---: | ---: | ---: |\n");
    for comparison in &comparisons {
        let empty = BTreeMap::new();
        let delta = comparison.rpc_delta_per_pair.as_ref().unwrap_or(&empty);
        writeln!(
            report,
            "| {} | {} / {} | {} | {} |",
            cell(&comparison.case_id),
            rpc_median(delta, "client_requests"),
            rpc_median(delta, "upstream_requests"),
            rpc_median(delta, "client_response_body_bytes"),
            rpc_median(delta, "bal_observed_body_bytes")
        )?;
    }
    report.push_str("\nPer-method RPC totals, per-attempt distributions and paired deltas, upstream spans, payload sizes and process-exit versus cleanup snapshots are in `summary.json`; raw exchanges are in `rpc-events.jsonl`. All RPCs have byte and completion metrics; only standalone BAL responses retain bodies for JSON-RPC classification and result sizes. Other responses remain unobserved, so error totals cover HTTP/transport issues and analyzed BAL errors. BAL exchange bytes count each HTTP body once; passthrough mixed batches include other methods. BAL injection requires a standalone request; rejected BAL batches invalidate the sample. Incomplete body sizes are observed bytes, not complete response sizes. Request spans are not summed as process wall time.\n\n");
    if !has_runner {
        report.push_str("\nNo common projection: measurement runner metadata was not recorded in `manifest.json`. Diagnostics above remain available; rerun sampling to record the measurement machine.\n");
    } else if !has_common_projection {
        report.push_str("\nNo common projection: no eligible performance measurements. No zero-duration placeholder was emitted.\n");
    }
    report.push_str("\nNo unconditional speedup is inferred from censored, unknown or correctness-blocked attempts. Server BAL source and cache behavior require independent evidence.\n");
    fs::write(output_dir.join("report.md"), report)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Sample, common_projection, compare_samples, group_samples, report};
    use crate::bal::{
        ActualPath, Arm, Binary, BuildIdentity,
        proxy::{BodyMetrics, RpcCall, RpcEvent, RpcResponse, Snapshot},
        write_json,
    };
    use foundry_bench::results::RunnerMetadata;
    use serde_json::{Value, json};
    use std::fs;

    fn sample(path: ActualPath, seconds: Option<f64>) -> Sample {
        Sample {
            schema_version: 1,
            id: "sample".into(),
            case_id: "middle".into(),
            phase: "measured".into(),
            round: 0,
            order: 0,
            arm: Arm::Auto,
            actual_path: path,
            exit_code: Some(0),
            timed_out: seconds.is_none(),
            wall_time_seconds: seconds,
            observed_duration_seconds: seconds.unwrap_or(10.0),
            stdout_sha256: "hash".into(),
            local_gas: Some(21_000),
            execution_success: Some(true),
            correctness: "equivalent".into(),
            fault: None,
            synthetic: false,
            rpc_at_exit: Snapshot::default(),
            rpc: Snapshot::default(),
            bal_events: vec![],
        }
    }

    #[test]
    fn round_trip_preserves_censoring_and_fallback_denominator() {
        let samples = vec![
            paired_samples().remove(0),
            sample(ActualPath::ReplayAfterProbe, Some(5.0)),
            sample(ActualPath::Failed, None),
        ];
        let encoded = serde_json::to_value(&samples).unwrap();
        let decoded = serde_json::from_value::<Vec<Sample>>(encoded.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
        let groups = group_samples(&decoded);
        let overall = &groups[0];
        assert_eq!(
            (overall.attempts, overall.completed, overall.eligible, overall.censored),
            (3, 2, 2, 1)
        );
        assert_eq!(overall.completed_wall_seconds.as_ref().unwrap().median, 3.0);
        assert_eq!(overall.actual_paths["replay_after_probe"], 1);
        assert_eq!(groups[1].attempts, 1);
        let common = common_projection(&groups, "sha", RunnerMetadata::default()).unwrap();
        assert_eq!(common.benchmarks[0].counters["scheduled_attempts"].value, 3.0);
        assert_eq!(common.benchmarks[0].wall_time.value, 3.0);
    }

    #[test]
    fn rpc_totals_preserve_counts_without_snapshot_analysis_state() {
        let mut first = sample(ActualPath::ReplayAfterProbe, Some(1.0));
        first.rpc_at_exit = Snapshot {
            client_requests_by_method: [("eth_getStorageAt".into(), 3)].into(),
            upstream_requests_by_method: [("eth_getStorageAt".into(), 2)].into(),
            client_http_exchanges: 3,
            upstream_http_exchanges: 2,
            injected_responses: 1,
            client_response_body_bytes: 120,
            upstream_response_body_bytes: 80,
            repeated_requests: 2,
            errors: 1,
            active_http_exchanges: 1,
            response_analysis_complete: false,
        };
        let mut second = first.clone();
        second.rpc_at_exit.client_requests_by_method.insert("eth_getBlockAccessList".into(), 1);
        second.rpc_at_exit.upstream_requests_by_method.insert("eth_getBlockAccessList".into(), 1);
        second.rpc_at_exit.response_analysis_complete = true;
        first.rpc = first.rpc_at_exit.clone();
        second.rpc = second.rpc_at_exit.clone();
        second.rpc.client_response_body_bytes += 7;
        second.rpc.upstream_response_body_bytes += 5;

        let groups = serde_json::to_value(group_samples(&[first, second])).unwrap();
        let mut expected = json!({
            "client_requests_by_method": {"eth_getStorageAt":6,"eth_getBlockAccessList":1},
            "upstream_requests_by_method": {"eth_getStorageAt":4,"eth_getBlockAccessList":1},
            "client_http_exchanges":6,
            "upstream_http_exchanges":4,
            "injected_responses":2,
            "client_response_body_bytes":240,
            "upstream_response_body_bytes":160,
            "repeated_requests":4,
            "errors":2,
            "active_http_exchanges":2,
        });
        assert_eq!(groups[0]["rpc_at_exit"], expected);
        expected["client_response_body_bytes"] = json!(247);
        expected["upstream_response_body_bytes"] = json!(165);
        assert_eq!(groups[0]["rpc_after_cleanup"], expected);
    }

    #[test]
    fn all_failed_has_no_common_projection_or_fake_zero() {
        let root = tempfile::tempdir().unwrap();
        write_json(
            &root.path().join("manifest.json"),
            &json!({"binary":{"sha256":"a".repeat(64)},"build":{"source_sha":"sha"},
                "runner":RunnerMetadata::default()}),
        )
        .unwrap();
        fs::write(
            root.path().join("samples.jsonl"),
            serde_json::to_string(&sample(ActualPath::Failed, None)).unwrap(),
        )
        .unwrap();
        report(root.path()).unwrap();
        assert!(!root.path().join("common-results.json").exists());
        let summary =
            serde_json::from_slice::<Value>(&fs::read(root.path().join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(summary["no_common_projection"], true);
        assert!(summary["groups"][0]["completed_wall_seconds"].is_null());
    }

    #[test]
    fn blocked_warmup_and_validation_never_become_valid_measurements() {
        let mut blocked = sample(ActualPath::BalHit, Some(9.0));
        blocked.correctness = "correctness_blocked".into();
        let mut warmup = sample(ActualPath::BalHit, Some(0.1));
        warmup.phase = "warmup".into();
        let groups = group_samples(&[blocked, warmup]);
        assert_eq!(groups[0].attempts, 1);
        assert_eq!(groups[0].correctness_blocked, 1);
        assert_eq!(groups[0].completed_wall_seconds.as_ref().unwrap().median, 9.0);
        assert!(common_projection(&groups, "sha", RunnerMetadata::default()).is_none());
    }

    #[test]
    fn missing_runner_preserves_diagnostics_but_removes_common_projection() {
        let root = tempfile::tempdir().unwrap();
        let mut manifest = paired_manifest();
        manifest["binary"] = json!({"sha256":"a".repeat(64)});
        write_json(&root.path().join("manifest.json"), &manifest).unwrap();
        let samples = paired_samples()
            .iter()
            .map(|sample| serde_json::to_string(sample).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(root.path().join("samples.jsonl"), samples).unwrap();
        fs::write(root.path().join("common-results.json"), "stale report-host metadata").unwrap();

        report(root.path()).unwrap();

        assert!(!root.path().join("common-results.json").exists());
        let summary =
            serde_json::from_slice::<Value>(&fs::read(root.path().join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(summary["no_common_projection"], true);
        assert_eq!(summary["comparisons"][0]["valid_pairs"], 2);
        assert_eq!(summary["comparisons"][0]["speedup"], 3.0);
        let markdown = fs::read_to_string(root.path().join("report.md")).unwrap();
        assert!(markdown.contains("measurement runner metadata was not recorded"));
        assert!(!markdown.contains("no eligible performance measurements"));
    }

    #[test]
    fn malformed_runner_metadata_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        for runner in [
            Value::Null,
            json!({}),
            json!({"os":"linux","arch":"x86_64","logical_cpus":"8"}),
            json!({"os":"","arch":"x86_64","logical_cpus":8}),
            json!({"os":"linux","arch":"","logical_cpus":8}),
            json!({"os":"linux","arch":"x86_64","logical_cpus":0}),
            json!({"os":"linux","arch":"x86_64","logical_cpus":8,"image":""}),
        ] {
            write_json(
                &root.path().join("manifest.json"),
                &json!({"binary":{"sha256":"a".repeat(64)},"runner":runner}),
            )
            .unwrap();
            let error = report(root.path()).unwrap_err();
            assert!(error.to_string().contains("invalid measurement runner metadata"));
            assert!(!root.path().join("report.md").exists());
            assert!(!root.path().join("common-results.json").exists());
        }
    }

    #[test]
    fn report_rejects_missing_or_invalid_rpc_counters() {
        let valid = serde_json::to_value(&paired_samples()[0]).unwrap();
        for (path, field) in [
            ("/rpc_at_exit", "client_response_body_bytes"),
            ("/rpc", "errors"),
            ("/bal_events/0/response", "body_bytes"),
        ] {
            for replacement in [None, Some(json!("123")), Some(Value::Null)] {
                let root = tempfile::tempdir().unwrap();
                write_json(
                    &root.path().join("manifest.json"),
                    &json!({"binary":{"sha256":"a".repeat(64)}}),
                )
                .unwrap();
                let mut malformed = valid.clone();
                let fields = malformed.pointer_mut(path).unwrap().as_object_mut().unwrap();
                let expected = if let Some(value) = replacement {
                    fields.insert(field.into(), value);
                    "invalid type".to_owned()
                } else {
                    fields.remove(field);
                    format!("missing field `{field}`")
                };
                fs::write(
                    root.path().join("samples.jsonl"),
                    serde_json::to_string(&malformed).unwrap(),
                )
                .unwrap();

                let error = report(root.path()).unwrap_err();
                assert!(error.to_string().contains(&expected), "{error}");
                assert!(!root.path().join("summary.json").exists());
                assert!(!root.path().join("report.md").exists());
            }
        }
    }

    fn paired_samples() -> Vec<Sample> {
        (0..2)
            .flat_map(|round| {
                let mut auto = sample(ActualPath::BalHit, Some(1.0 + 2.0 * round as f64));
                auto.round = round;
                auto.rpc_at_exit = Snapshot {
                    client_requests_by_method: [
                        ("eth_getBlockAccessList".into(), 1),
                        ("eth_getStorageAt".into(), 3),
                    ]
                    .into(),
                    upstream_requests_by_method: [
                        ("eth_getBlockAccessList".into(), 1),
                        ("eth_getStorageAt".into(), 3),
                    ]
                    .into(),
                    client_response_body_bytes: 120,
                    ..Snapshot::default()
                };
                auto.bal_events = vec![RpcEvent {
                    exchange_id: 0,
                    batch: false,
                    calls: vec![RpcCall {
                        method: "eth_getBlockAccessList".into(),
                        id: Some(json!(1)),
                        params: json!(["0x1"]),
                        forwarded: true,
                        injected: false,
                        repeated: false,
                        response: RpcResponse::Unobserved,
                        error_code: None,
                        result_json_bytes: None,
                    }],
                    client_request_body_bytes: 0,
                    upstream_request_body_bytes: 0,
                    upstream_http_exchanges: 1,
                    http_status: None,
                    upstream_http_status: None,
                    response: BodyMetrics {
                        body_bytes: 100,
                        cleanup_body_bytes: 4,
                        ..BodyMetrics::default()
                    },
                    upstream: None,
                    response_transformed: false,
                    issues: Vec::new(),
                }];
                let mut replay = sample(ActualPath::ReplayNoProbe, Some(5.0 + 2.0 * round as f64));
                replay.arm = Arm::Replay;
                replay.round = round;
                replay.rpc_at_exit = Snapshot {
                    client_requests_by_method: [("eth_getStorageAt".into(), 10)].into(),
                    upstream_requests_by_method: [("eth_getStorageAt".into(), 10)].into(),
                    client_response_body_bytes: 300,
                    ..Snapshot::default()
                };
                [auto, replay]
            })
            .collect()
    }

    fn paired_manifest() -> Value {
        json!({"rounds":2,"round_offset":0,"panel":{"cases":[{"id":"middle"}]}})
    }

    #[test]
    fn paired_comparison_reports_time_and_rpc_costs_without_summing_parallel_spans() {
        let mut samples = paired_samples();
        samples[0].actual_path = ActualPath::ReplayAfterProbe;
        let comparisons = compare_samples(&samples, &paired_manifest());
        let comparison = &comparisons[0];
        assert!(comparison.complete);
        assert_eq!((comparison.valid_pairs, comparison.scheduled_pairs), (2, 2));
        assert_eq!(comparison.auto_wall_seconds.as_ref().unwrap().median, 2.0);
        assert_eq!(comparison.replay_wall_seconds.as_ref().unwrap().median, 6.0);
        assert_eq!(comparison.paired_wall_delta_seconds.as_ref().unwrap().median, -4.0);
        assert_eq!(comparison.speedup, Some(3.0));
        let rpc = comparison.rpc_delta_per_pair.as_ref().unwrap();
        assert_eq!(rpc["client_requests"].median, -6.0);
        assert_eq!(rpc["upstream_requests_by_method.eth_getStorageAt"].median, -7.0);
        assert_eq!(rpc["upstream_requests_by_method.eth_getBlockAccessList"].median, 1.0);
        assert_eq!(rpc["client_response_body_bytes"].median, -180.0);
        assert_eq!(rpc["bal_observed_body_bytes"].median, 100.0);
        assert_eq!(rpc["bal_cleanup_body_bytes"].median, 4.0);
        let groups = group_samples(&samples);
        assert_eq!(groups[0].rpc_per_attempt["client_requests"].count, 2);
        assert_eq!(groups[0].rpc_per_attempt["client_requests"].median, 4.0);
    }

    #[test]
    fn failed_censored_unknown_and_mismatched_pairs_never_publish_speedup() {
        for failure in ["exit", "timeout", "unknown", "blocked", "trace", "gas", "status"] {
            let mut samples = paired_samples();
            match failure {
                "exit" => samples[0].exit_code = Some(1),
                "timeout" => {
                    samples[0].timed_out = true;
                    samples[0].wall_time_seconds = None;
                }
                "unknown" => samples[0].actual_path = ActualPath::Unknown,
                "blocked" => samples[0].correctness = "correctness_blocked".into(),
                "trace" => samples[0].stdout_sha256 = "different".into(),
                "gas" => samples[0].local_gas = Some(30_000),
                "status" => samples[0].execution_success = Some(false),
                _ => unreachable!(),
            }
            let comparisons = compare_samples(&samples, &paired_manifest());
            let comparison = &comparisons[0];
            assert_eq!((comparison.valid_pairs, comparison.invalid_pairs), (1, 1), "{failure}");
            assert!(!comparison.complete, "{failure}");
            assert!(comparison.speedup.is_none(), "{failure}");
            assert!(comparison.paired_wall_delta_seconds.is_none(), "{failure}");
            assert!(comparison.rpc_delta_per_pair.is_none(), "{failure}");
        }
    }

    #[test]
    fn missing_duplicate_and_unscheduled_attempts_cannot_shrink_the_denominator() {
        let complete = paired_samples();
        let missing_round = compare_samples(&complete[..2], &paired_manifest());
        assert_eq!((missing_round[0].valid_pairs, missing_round[0].scheduled_pairs), (1, 2));
        assert_eq!(missing_round[0].missing_pairs, 1);
        assert!(missing_round[0].speedup.is_none());
        let absent = compare_samples(&[], &paired_manifest());
        assert_eq!(absent[0].missing_pairs, 2);
        assert!(absent[0].speedup.is_none());
        let mut duplicate = complete.clone();
        duplicate.push(complete[0].clone());
        let comparisons = compare_samples(&duplicate, &paired_manifest());
        assert_eq!(comparisons[0].duplicate_pairs, 1);
        assert!(comparisons[0].speedup.is_none());
        let mut unexpected = complete;
        let mut extra = unexpected[0].clone();
        extra.round = 9;
        unexpected.push(extra);
        let comparisons = compare_samples(&unexpected, &paired_manifest());
        assert_eq!(comparisons[0].unexpected_rounds, 1);
        assert!(comparisons[0].speedup.is_none());
    }

    #[test]
    fn warmup_and_unproven_schedule_do_not_produce_comparisons() {
        let samples = paired_samples();
        let unknown_schedule = compare_samples(&samples, &json!({}));
        assert!(unknown_schedule[0].speedup.is_none());
        let mut manifest = paired_manifest();
        manifest["round_offset"] = json!(4);
        let mut measured = samples.clone();
        for sample in &mut measured {
            sample.round += 4;
        }
        for mut sample in samples {
            sample.phase = "warmup".into();
            sample.wall_time_seconds = Some(100.0);
            measured.push(sample);
        }
        let comparisons = compare_samples(&measured, &manifest);
        assert_eq!(comparisons[0].speedup, Some(3.0));
        assert_eq!(comparisons[0].observed_auto_attempts, 2);
    }

    #[test]
    fn report_requires_single_binary_provenance_and_rejects_legacy_controls() {
        let root = tempfile::tempdir().unwrap();
        for manifest in [
            json!({"build":{"auto":{"sha256":"a".repeat(64)},"replay":{"sha256":"b".repeat(64)}}}),
            json!({}),
            json!({"binary":{"sha256":"a".repeat(64)},"build":{"cast":{"sha256":"b".repeat(64)}}}),
        ] {
            write_json(&root.path().join("manifest.json"), &manifest).unwrap();
            assert!(report(root.path()).is_err());
            assert!(!root.path().join("report.md").exists());
        }
        for manifest in [
            json!({"binary":{"sha256":"a".repeat(64)},"build":null}),
            json!({"build":{"cast":{"sha256":"a".repeat(64)}}}),
        ] {
            write_json(&root.path().join("manifest.json"), &manifest).unwrap();
            report(root.path()).unwrap();
            assert!(root.path().join("report.md").exists());
        }
    }

    #[test]
    fn direct_and_aggregated_reports_accept_runner_binary_types_and_preserve_pairs() {
        let root = tempfile::tempdir().unwrap();
        let binary = Binary {
            path: root.path().join("cast"),
            sha256: "a".repeat(64),
            version: "cast fixture".into(),
        };
        let build = BuildIdentity {
            schema_version: 1,
            source_sha: "b".repeat(40),
            cargo_lock_sha256: "c".repeat(64),
            rustc: "rustc fixture".into(),
            build_argv: [
                "cargo",
                "build",
                "--locked",
                "--profile",
                "profiling",
                "-p",
                "cast",
                "--bin",
                "cast",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            build_env: Default::default(),
            cast: binary.clone(),
        };
        let mut direct = paired_manifest();
        let runner = json!({
            "os": "measurement-os", "arch": "measurement-arch",
            "image": "measurement-image", "logical_cpus": 37,
        });
        direct["schema_version"] = json!(1);
        direct["binary"] = serde_json::to_value(binary).unwrap();
        direct["build"] = Value::Null;
        direct["runner"] = runner.clone();
        let mut aggregate = paired_manifest();
        aggregate["schema_version"] = json!(1);
        aggregate["build"] = serde_json::to_value(build).unwrap();
        aggregate["schedule"] = json!("/retained-artifacts/schedule.json");
        aggregate["runner"] = runner.clone();
        for manifest in [direct, aggregate] {
            write_json(&root.path().join("manifest.json"), &manifest).unwrap();
            let samples = paired_samples()
                .into_iter()
                .map(|sample| {
                    let mut value = serde_json::to_value(sample).unwrap();
                    // The wrapper adds source references without changing sample semantics.
                    if manifest.get("schedule").is_some() {
                        value["source_sample_id"] = value["id"].clone();
                        value["source_run_directory"] = json!("/retained-artifacts/measured-000");
                    }
                    serde_json::to_string(&value).unwrap()
                })
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(root.path().join("samples.jsonl"), samples).unwrap();
            report(root.path()).unwrap();
            let common = serde_json::from_slice::<Value>(
                &fs::read(root.path().join("common-results.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(common["runner"], runner);
            assert_eq!(common["benchmarks"][0]["wall_time"]["value"], 2.0);
            let summary = serde_json::from_slice::<Value>(
                &fs::read(root.path().join("summary.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(summary["comparisons"][0]["valid_pairs"], 2);
            assert_eq!(summary["comparisons"][0]["speedup"], 3.0);
            assert_eq!(
                summary["comparisons"][0]["rpc_delta_per_pair"]["client_requests"]["median"],
                -6.0
            );
        }
    }
}
