#!/usr/bin/env python3
"""Build Foundry benchmark manifests and ingest trusted artifacts into ClickHouse."""

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import pathlib
import platform
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request


MAX_JSON_BYTES = 16 * 1024 * 1024
MAX_SAMPLES = 10_000
RUN_SCHEMA = "foundry-benchmark-run/v1"
SUITE_SCHEMA = "foundry-benchmark-suite/v1"
INGESTER_VERSION = 1
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
IDENTIFIER_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
EXPECTED_SUITES = {
    "ci:test": {"forge_test", "forge_fuzz_test"},
    "ci:isolate": {"forge_isolate_test"},
    "ci:build": {"forge_build_no_cache", "forge_build_with_cache"},
    "ci:coverage": {"forge_coverage"},
}


def _reject_duplicate_keys(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError(f"duplicate JSON key: {key}")
        obj[key] = value
    return obj


def _reject_non_finite(value):
    raise ValueError(f"non-finite JSON number: {value}")


def load_json(path):
    path = pathlib.Path(path)
    size = path.stat().st_size
    if size > MAX_JSON_BYTES:
        raise ValueError(f"{path} exceeds {MAX_JSON_BYTES} bytes")
    with path.open(encoding="utf-8") as handle:
        return json.load(
            handle,
            object_pairs_hook=_reject_duplicate_keys,
            parse_constant=_reject_non_finite,
        )


def _command_version(*command):
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout.strip()


def _cpu_model():
    if sys.platform == "darwin":
        return _command_version("sysctl", "-n", "machdep.cpu.brand_string")
    cpuinfo = pathlib.Path("/proc/cpuinfo")
    if cpuinfo.exists():
        for line in cpuinfo.read_text(encoding="utf-8").splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown"


def build_manifest(fragment_paths, baseline, candidate):
    if not SHA_RE.fullmatch(baseline) or not SHA_RE.fullmatch(candidate):
        raise ValueError("baseline and candidate must be full lowercase Git commit SHAs")

    suites = []
    expected_keys = set()
    for path in fragment_paths:
        suite = load_json(path)
        if suite.get("schema") != SUITE_SCHEMA:
            raise ValueError(f"unsupported suite schema in {path}")
        if not isinstance(suite.get("suite"), str) or not isinstance(suite.get("cases"), list):
            raise ValueError(f"invalid suite manifest in {path}")
        for case in suite["cases"]:
            case_id = case.get("id")
            if not isinstance(case_id, str) or not case_id:
                raise ValueError(f"invalid benchmark case in {path}")
            if case_id in expected_keys:
                raise ValueError(f"duplicate benchmark case: {case_id}")
            if set(case.get("versions", [])) != {"master", "local"}:
                raise ValueError(f"benchmark case does not contain both comparison sides: {case_id}")
            expected_keys.add(case_id)
        suites.append(suite)

    if not expected_keys:
        raise ValueError("benchmark manifest contains no cases")

    repository = os.environ["GITHUB_REPOSITORY"]
    server_url = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    run_id = int(os.environ["GITHUB_RUN_ID"])
    run_attempt = int(os.environ["GITHUB_RUN_ATTEMPT"])
    branch = os.environ.get("GITHUB_REF_NAME")
    now = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
    return {
        "schema": RUN_SCHEMA,
        "project": "foundry",
        "run": {
            "repository": repository,
            "workflow": os.environ.get("GITHUB_WORKFLOW", "Foundry Benchmarks"),
            "workflow_file": ".github/workflows/benchmarks.yml",
            "run_id": run_id,
            "run_attempt": run_attempt,
            "run_url": f"{server_url}/{repository}/actions/runs/{run_id}",
            "event": os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch"),
            "branch": branch,
            "generated_at": now,
        },
        "comparison": {
            "kind": "source_vs_master",
            "baseline_ref": "master",
            "baseline_commit": baseline,
            "candidate_ref": branch,
            "candidate_commit": candidate,
        },
        "environment": {
            "build_profile": os.environ.get("FOUNDRY_BENCH_LOCAL_BUILD_PROFILE", "profiling"),
            "runner_os": os.environ.get("RUNNER_OS", platform.system()),
            "runner_arch": os.environ.get("RUNNER_ARCH", platform.machine()),
            "runner_name": os.environ.get("RUNNER_NAME"),
            "cpu_model": _cpu_model(),
            "cpu_count": os.cpu_count(),
            "rustc": _command_version("rustc", "--version"),
            "hyperfine": _command_version("hyperfine", "--version"),
        },
        "expected_result_keys": sorted(expected_keys),
        "suites": suites,
    }


def _require_string(value, field, *, allow_empty=False):
    if not isinstance(value, str) or (not allow_empty and not value):
        raise ValueError(f"{field} must be a non-empty string")
    return value


def _require_sha(value, field):
    value = _require_string(value, field)
    if not SHA_RE.fullmatch(value):
        raise ValueError(f"{field} must be a full lowercase Git commit SHA")
    return value


def _number(value, field, *, nullable=False):
    if value is None and nullable:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be numeric")
    value = float(value)
    if not math.isfinite(value) or value < 0:
        raise ValueError(f"{field} must be finite and non-negative")
    return value


def _canonical_hash(value):
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def _trusted_metadata():
    required = [
        "TRUSTED_REPOSITORY",
        "TRUSTED_WORKFLOW_FILE",
        "TRUSTED_DEFAULT_BRANCH",
        "TRUSTED_RUN_ID",
        "TRUSTED_RUN_ATTEMPT",
        "TRUSTED_RUN_URL",
        "TRUSTED_HEAD_SHA",
        "TRUSTED_RUN_STARTED_AT",
    ]
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        raise ValueError(f"missing trusted workflow metadata: {', '.join(missing)}")
    started = dt.datetime.fromisoformat(
        os.environ["TRUSTED_RUN_STARTED_AT"].replace("Z", "+00:00")
    ).astimezone(dt.timezone.utc)
    return {
        "repository": os.environ["TRUSTED_REPOSITORY"],
        "workflow_file": os.environ["TRUSTED_WORKFLOW_FILE"],
        "default_branch": os.environ["TRUSTED_DEFAULT_BRANCH"],
        "run_id": int(os.environ["TRUSTED_RUN_ID"]),
        "run_attempt": int(os.environ["TRUSTED_RUN_ATTEMPT"]),
        "run_url": os.environ["TRUSTED_RUN_URL"],
        "head_sha": _require_sha(os.environ["TRUSTED_HEAD_SHA"], "trusted head SHA"),
        "branch": os.environ.get("TRUSTED_HEAD_BRANCH") or None,
        "started_at": started.strftime("%Y-%m-%d %H:%M:%S.%f")[:-3],
        "check_baseline_ancestor": True,
    }


def build_rows(artifact_dir, trusted=None):
    artifact_dir = pathlib.Path(artifact_dir)
    trusted = trusted or _trusted_metadata()
    manifest = load_json(artifact_dir / "benchmark-manifest.json")
    baseline_summary = load_json(artifact_dir / "base-summary.json")
    candidate_summary = load_json(artifact_dir / "candidate-summary.json")

    if manifest.get("schema") != RUN_SCHEMA or manifest.get("project") != "foundry":
        raise ValueError("unsupported benchmark run manifest")
    run = manifest.get("run", {})
    comparison = manifest.get("comparison", {})
    if run.get("repository") != trusted["repository"]:
        raise ValueError("artifact repository does not match triggering workflow")
    if run.get("workflow_file") != trusted["workflow_file"]:
        raise ValueError("artifact came from an unexpected workflow")
    if run.get("event") != "workflow_dispatch":
        raise ValueError("artifact came from an unexpected event")
    if run.get("run_id") != trusted["run_id"] or run.get("run_attempt") != trusted["run_attempt"]:
        raise ValueError("artifact run identity does not match triggering workflow")

    baseline_commit = _require_sha(comparison.get("baseline_commit"), "baseline commit")
    candidate_commit = _require_sha(comparison.get("candidate_commit"), "candidate commit")
    if candidate_commit != trusted["head_sha"]:
        raise ValueError("candidate commit does not match triggering workflow")
    if comparison.get("kind") != "source_vs_master":
        raise ValueError("unsupported comparison kind")
    if comparison.get("baseline_ref") != trusted["default_branch"]:
        raise ValueError("baseline ref does not match the default branch")
    if comparison.get("candidate_ref") != trusted["branch"]:
        raise ValueError("candidate ref does not match the triggering branch")
    if trusted.get("check_baseline_ancestor"):
        ancestor = subprocess.run(
            ["git", "merge-base", "--is-ancestor", baseline_commit, "HEAD"],
            check=False,
        )
        if ancestor.returncode != 0:
            raise ValueError("baseline commit is not an ancestor of the current default branch")

    expected = manifest.get("expected_result_keys")
    if not isinstance(expected, list) or not expected or any(not isinstance(key, str) for key in expected):
        raise ValueError("manifest has invalid expected result keys")
    if len(expected) != len(set(expected)):
        raise ValueError("manifest contains duplicate expected result keys")
    expected_set = set(expected)
    if set(baseline_summary) != expected_set or set(candidate_summary) != expected_set:
        raise ValueError("baseline and candidate summaries must exactly match the manifest")

    cases = {}
    suites = manifest.get("suites", [])
    if not isinstance(suites, list):
        raise ValueError("manifest suites must be an array")
    seen_suites = set()
    for suite in suites:
        if suite.get("schema") != SUITE_SCHEMA:
            raise ValueError("unsupported suite schema")
        suite_name = suite.get("suite")
        if suite_name in seen_suites:
            raise ValueError(f"duplicate benchmark suite: {suite_name}")
        if suite_name not in EXPECTED_SUITES:
            raise ValueError(f"unexpected benchmark suite: {suite_name}")
        seen_suites.add(suite_name)
        suite_benchmarks = {case.get("benchmark") for case in suite.get("cases", [])}
        if suite_benchmarks != EXPECTED_SUITES[suite_name]:
            raise ValueError(f"benchmark suite is incomplete: {suite_name}")
        for case in suite.get("cases", []):
            case_id = case.get("id")
            if case_id in cases:
                raise ValueError(f"duplicate benchmark case: {case_id}")
            cases[case_id] = case
    if seen_suites != set(EXPECTED_SUITES):
        raise ValueError("benchmark manifest does not contain every required suite")
    if set(cases) != expected_set:
        raise ValueError("suite cases must exactly match expected result keys")

    rows = []
    environment = manifest.get("environment", {})
    cpu_count = environment.get("cpu_count")
    if isinstance(cpu_count, bool) or not isinstance(cpu_count, int) or not 1 <= cpu_count <= 65535:
        raise ValueError("CPU count must be an integer between 1 and 65535")
    for side_role, summary, tool_ref, tool_commit, tool_channel in [
        ("baseline", baseline_summary, comparison.get("baseline_ref"), baseline_commit, "master"),
        ("candidate", candidate_summary, comparison.get("candidate_ref"), candidate_commit, "local"),
    ]:
        for case_id in sorted(expected):
            case = cases[case_id]
            result = summary[case_id]
            if not isinstance(result, dict):
                raise ValueError(f"result must be an object: {case_id}")
            workload_commit = _require_sha(case.get("workload_commit"), "workload commit")
            samples = result.get("times")
            if not isinstance(samples, list) or not samples or len(samples) > MAX_SAMPLES:
                raise ValueError(f"invalid timing samples: {case_id}")
            samples = [_number(value, f"{case_id}.times") for value in samples]
            exit_codes = result.get("exit_codes") or []
            if not isinstance(exit_codes, list) or any(
                isinstance(code, bool)
                or not isinstance(code, int)
                or not -(2**31) <= code < 2**31
                for code in exit_codes
            ):
                raise ValueError(f"invalid exit codes: {case_id}")
            symbolic = result.get("symbolic") or {}
            if not isinstance(symbolic, dict) or any(
                isinstance(value, bool)
                or not isinstance(value, int)
                or not 0 <= value < 2**64
                for value in symbolic.values()
            ):
                raise ValueError(f"invalid symbolic counters: {case_id}")

            configuration = {
                "benchmark": case.get("benchmark"),
                "workload_repository": case.get("workload_repository"),
                "workload_requested_ref": case.get("workload_requested_ref"),
                "workload_commit": workload_commit,
                "arguments": case.get("arguments"),
            }
            identity = {
                "project": "foundry",
                "repository": trusted["repository"],
                "workflow_file": run["workflow_file"],
                "run_id": trusted["run_id"],
                "run_attempt": trusted["run_attempt"],
                "side_role": side_role,
                "benchmark_case_id": case_id,
                "workload_repository": case.get("workload_repository"),
                "workload_commit": workload_commit,
            }
            rows.append({
                "project": "foundry",
                "github_repository": trusted["repository"],
                "workflow_file": run["workflow_file"],
                "workflow_run_id": trusted["run_id"],
                "workflow_run_attempt": trusted["run_attempt"],
                "workflow_run_url": trusted["run_url"],
                "run_started_at": trusted["started_at"],
                "branch": trusted.get("branch"),
                "pr_number": None,
                "comparison_kind": comparison["kind"],
                "side_role": side_role,
                "tool_channel": tool_channel,
                "tool_ref": _require_string(tool_ref, f"{side_role} ref"),
                "tool_commit": tool_commit,
                "benchmark_case_id": case_id,
                "benchmark_name": _require_string(case.get("benchmark"), "benchmark name"),
                "benchmark_config_hash": _canonical_hash(configuration),
                "workload_repository": _require_string(
                    case.get("workload_repository"), "workload repository"
                ),
                "workload_requested_ref": _require_string(
                    case.get("workload_requested_ref"), "workload requested ref"
                ),
                "workload_commit": workload_commit,
                "mean_seconds": _number(result.get("mean"), f"{case_id}.mean"),
                "stddev_seconds": _number(
                    result.get("stddev"), f"{case_id}.stddev", nullable=True
                ),
                "median_seconds": _number(result.get("median"), f"{case_id}.median"),
                "user_seconds": _number(result.get("user"), f"{case_id}.user"),
                "system_seconds": _number(result.get("system"), f"{case_id}.system"),
                "min_seconds": _number(result.get("min"), f"{case_id}.min"),
                "max_seconds": _number(result.get("max"), f"{case_id}.max"),
                "samples_seconds": samples,
                "exit_codes": exit_codes,
                "parameters_json": json.dumps(
                    result.get("parameters"), sort_keys=True, separators=(",", ":")
                ) if result.get("parameters") is not None else "",
                "counters_u64": symbolic,
                "metrics_f64": {},
                "build_profile": _require_string(environment.get("build_profile"), "build profile"),
                "runner_os": _require_string(environment.get("runner_os"), "runner OS"),
                "runner_arch": _require_string(environment.get("runner_arch"), "runner architecture"),
                "cpu_model": _require_string(environment.get("cpu_model"), "CPU model"),
                "cpu_count": cpu_count,
                "rustc_version": _require_string(environment.get("rustc"), "rustc version"),
                "hyperfine_version": _require_string(
                    environment.get("hyperfine"), "hyperfine version"
                ),
                "manifest_schema": RUN_SCHEMA,
                "ingester_version": INGESTER_VERSION,
                "result_id": _canonical_hash(identity),
            })
    return rows


def upload_rows(rows):
    host = os.environ.get("CLICKHOUSE_HOST")
    user = os.environ.get("CLICKHOUSE_USER")
    password = os.environ.get("CLICKHOUSE_PASSWORD")
    if not host or not user or not password:
        raise ValueError(
            "CLICKHOUSE_HOST, CLICKHOUSE_USER, and CLICKHOUSE_PASSWORD are required"
        )
    base_url = host if "://" in host else f"https://{host}:8443"
    parsed = urllib.parse.urlsplit(base_url)
    if parsed.scheme != "https" or not parsed.netloc:
        raise ValueError("CLICKHOUSE_HOST must identify an HTTPS endpoint")
    table = os.environ.get("CLICKHOUSE_TABLE", "benchmark_results")
    if not IDENTIFIER_RE.fullmatch(table):
        raise ValueError("CLICKHOUSE_TABLE must be an unquoted ClickHouse identifier")
    query = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    query.extend([
        ("database", os.environ.get("CLICKHOUSE_DATABASE", "default")),
        ("query", f"INSERT INTO {table} FORMAT JSONEachRow"),
    ])
    url = urllib.parse.urlunsplit(
        (parsed.scheme, parsed.netloc, parsed.path or "/", urllib.parse.urlencode(query), "")
    )
    body = "".join(
        json.dumps(row, separators=(",", ":")) + "\n" for row in rows
    ).encode()
    request = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Content-Type": "application/x-ndjson",
            "X-ClickHouse-User": user,
            "X-ClickHouse-Key": password,
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            response.read()
    except urllib.error.HTTPError as error:
        detail = error.read(4096).decode(errors="replace")
        raise RuntimeError(f"ClickHouse insert failed ({error.code}): {detail}") from error


def main():
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    manifest_parser = subparsers.add_parser("manifest")
    manifest_parser.add_argument("fragments", nargs="+")
    manifest_parser.add_argument("--output", required=True)
    manifest_parser.add_argument("--baseline", required=True)
    manifest_parser.add_argument("--candidate", required=True)

    rows_parser = subparsers.add_parser("rows")
    rows_parser.add_argument("artifact_dir")
    rows_parser.add_argument("--output", default="-")

    upload_parser = subparsers.add_parser("upload")
    upload_parser.add_argument("artifact_dir")

    args = parser.parse_args()
    if args.command == "manifest":
        manifest = build_manifest(args.fragments, args.baseline, args.candidate)
        output = pathlib.Path(args.output)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    elif args.command == "rows":
        rows = build_rows(args.artifact_dir)
        content = "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
        if args.output == "-":
            sys.stdout.write(content)
        else:
            pathlib.Path(args.output).write_text(content, encoding="utf-8")
    else:
        rows = build_rows(args.artifact_dir)
        upload_rows(rows)
        print(f"Inserted {len(rows)} benchmark rows into ClickHouse.")


if __name__ == "__main__":
    main()
