#!/usr/bin/env bash
set -euo pipefail

# Python provides portable hashing, exact source checks, and JSON provenance.
exec python3 - "${BASH_SOURCE[0]}" "$@" <<'PY'
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys


def fail(message):
    raise SystemExit(f"error: {message}")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(argv, cwd, env=None, capture=False):
    return subprocess.run(
        argv, cwd=cwd, env=env, check=True,
        stdout=subprocess.PIPE if capture else None,
        text=True,
    ).stdout


def git(*args, cwd=None):
    return run(["git", *args], cwd or repo, capture=True).strip()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def positive_integer(name, default, minimum=1):
    value = os.environ.get(name, str(default))
    if not value.isdecimal() or int(value) < minimum:
        fail(f"{name} must be an integer >= {minimum}")
    return int(value)


script = Path(sys.argv[1]).resolve()
repo = script.parents[2]
if len(sys.argv) > 2:
    if sys.argv[2:] != ["--help"]:
        fail("configuration uses environment variables; see --help")
    print("""Build unmodified Cast refs and alternate complete benchmark rounds.

Environment:
  BASE_REF                 Base git ref (default: origin/master).
  CANDIDATE_REF            Candidate git ref (default: HEAD).
  BENCH_ROOT               New artifact directory (default: unique /tmp directory).
  BUILD_ONLY=1             Build binaries without archive requests.
  PANEL_MANIFEST           Frozen panel JSON (required unless BUILD_ONLY=1).
  RPC_ENV                  Optional environment variable containing the HTTP RPC URL.
                           Omit to use configured RPC settings, then the repository archive endpoint.
  ROUNDS                   Measured rounds per ref (default: 10).
  WARMUP_ROUNDS            Warmup rounds per ref (default: 2).
  TIMEOUT_SECONDS          Child deadline (default: 120).
  INCLUDE_MISS=1           Include the controlled unsupported-BAL fallback arm.
  BAL_BENCH_FEATURES       Identical Cargo features for both refs (default: package defaults).

Requires git, Python 3, cargo and rustc. Uses --locked --profile profiling.
Both refs must support cast run --no-bal; auto and replay use the same binary.
Artifacts and detached worktrees are retained, including after failure.
""")
    raise SystemExit(0)

build_only = os.environ.get("BUILD_ONLY", "0")
if build_only not in ("0", "1"):
    fail("BUILD_ONLY must be 0 or 1")
include_miss = os.environ.get("INCLUDE_MISS", "0")
if include_miss not in ("0", "1"):
    fail("INCLUDE_MISS must be 0 or 1")
rounds = positive_integer("ROUNDS", 10)
warmups = positive_integer("WARMUP_ROUNDS", 2, 0)
timeout = positive_integer("TIMEOUT_SECONDS", 120)
panel = os.environ.get("PANEL_MANIFEST")
if build_only == "0":
    if not panel or not Path(panel).is_file():
        fail("PANEL_MANIFEST must name a frozen panel file (or set BUILD_ONLY=1)")
    panel = Path(panel).resolve()
rpc_env = os.environ.get("RPC_ENV")
if rpc_env and (not rpc_env.isidentifier() or not rpc_env.isascii()):
    fail("RPC_ENV must be an environment variable name")
if build_only == "0" and rpc_env and not os.environ.get(rpc_env):
    fail("the RPC_ENV variable is unset or empty")

# Resolve refs once; each distinct commit gets one unmodified worktree and binary.
refs = {
    "base": git("rev-parse", "--verify", f"{os.environ.get('BASE_REF', 'origin/master')}^{{commit}}"),
    "candidate": git("rev-parse", "--verify", f"{os.environ.get('CANDIDATE_REF', 'HEAD')}^{{commit}}"),
}
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
root = Path(os.environ.get("BENCH_ROOT", f"/tmp/foundry-pr-bal-bench-{stamp}")).resolve()
if root.exists():
    fail("BENCH_ROOT already exists; choose a new directory to preserve existing artifacts")
root.mkdir(parents=True)
# No RPC credentials or arbitrary FOUNDRY_/CARGO_ settings enter Cast builds.
# Cargo home/config and native-toolchain paths remain available, identically for both refs.
build_keys = (
    "PATH", "HOME", "USER", "TMPDIR", "TEMP", "TMP", "CARGO_HOME", "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_JOBS",
    "CC", "CXX", "AR", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "PKG_CONFIG_PATH",
    "SOURCE_DATE_EPOCH",
)
build_env = {key: os.environ[key] for key in build_keys if key in os.environ}
build_env["CARGO_INCREMENTAL"] = "0"
build_env["FOUNDRY_DISABLE_NIGHTLY_WARNING"] = "true"
build_argv = ["cargo", "build", "--locked", "--profile", "profiling", "-p", "cast", "--bin", "cast"]
features = os.environ.get("BAL_BENCH_FEATURES")
if features:
    build_argv.extend(["--features", features])


def verify_tree(tree, sha):
    if git("rev-parse", "HEAD", cwd=tree) != sha:
        fail("Cast build changed its pinned source commit")
    if git("status", "--porcelain", "--untracked-files=all", cwd=tree):
        fail("Cast build introduced source changes; only unmodified builds are comparable")


manifests = {}
binaries = {}
comparison_compiler = None
builds_by_sha = {}
builds = {"base": "base", "candidate": "base" if refs["base"] == refs["candidate"] else "candidate"}
write_json(root / "build-schedule.json", {
    "schema_version": 1,
    "refs": refs,
    "same_source_refs": refs["base"] == refs["candidate"],
    "builds": builds,
})
for label, sha in refs.items():
    build = root / label
    build.mkdir()
    manifest = build / "build.json"
    if sha in builds_by_sha:
        original_label = builds_by_sha[sha]
        manifest.write_bytes(manifests[original_label].read_bytes())
        manifests[label] = manifest
        binaries[label] = binaries[original_label]
        continue
    tree = build / "source"
    run(["git", "worktree", "add", "--detach", str(tree), sha], repo)
    verify_tree(tree, sha)
    lock_hash = digest(tree / "Cargo.lock")
    environment = dict(build_env, CARGO_TARGET_DIR=str(build / "target"))
    rustc = run(["rustc", "-Vv"], tree, environment, capture=True).strip()
    if comparison_compiler is not None and rustc != comparison_compiler:
        fail("base and candidate resolved different Rust toolchains")
    comparison_compiler = rustc
    run(build_argv, tree, environment)
    verify_tree(tree, sha)
    binary = build / "target" / "profiling" / "cast"
    help_text = run([str(binary), "run", "--help"], tree, environment, capture=True)
    if "--no-bal" not in help_text.split():
        fail(f"{label} Cast lacks --no-bal; select a ref containing PR #16931")
    write_json(manifest, {
        "schema_version": 1,
        "source_sha": sha,
        "cargo_lock_sha256": lock_hash,
        "rustc": rustc,
        "build_argv": build_argv,
        "build_env": environment,
        "cast": {
            "path": str(binary),
            "sha256": digest(binary),
            "version": run([str(binary), "--version"], tree, environment, capture=True).strip(),
        },
    })
    manifests[label] = manifest
    binaries[label] = binary
    builds_by_sha[sha] = label

if build_only == "1":
    print(f"Cast builds and provenance retained in {root}")
    raise SystemExit(0)

# Build the runner from this checkout, allowing the benchmark tooling under development.
runner_env = dict(build_env, CARGO_TARGET_DIR=str(root / "runner-target"))
runner_argv = [
    "cargo", "build", "--locked", "--profile", "profiling", "-p", "foundry-bench",
    "--bin", "foundry-cast-run-bench",
]
run(runner_argv, repo, runner_env)
runner = root / "runner-target" / "profiling" / "foundry-cast-run-bench"
changed = git("diff", "HEAD", "--name-only").splitlines()
untracked = git("ls-files", "--others", "--exclude-standard", "--", "benches").splitlines()
write_json(root / "runner-build.json", {
    "source_sha": git("rev-parse", "HEAD"),
    "changed_file_sha256": {
        name: digest(repo / name) if (repo / name).is_file() else None
        for name in sorted(set(changed + untracked))
    },
    "path": str(runner), "sha256": digest(runner),
    "rustc": run(["rustc", "-Vv"], repo, runner_env, capture=True).strip(),
    "build_argv": runner_argv, "build_env": runner_env,
})
frozen_panel = root / "panel.json"
frozen_panel.write_bytes(panel.read_bytes())
schedule = {
    "schema_version": 1,
    "refs": refs,
    "same_source_refs": refs["base"] == refs["candidate"],
    "builds": builds,
    "include_miss": include_miss == "1",
    "panel_sha256": digest(frozen_panel),
    "rounds": rounds,
    "warmup_rounds": warmups,
    "timeout_seconds": timeout,
    "rpc_env": rpc_env,
    "execution_order": [],
}


def execute(label, round_index, warmup):
    phase = "warmup" if warmup else "measured"
    output = root / "results" / label / f"{phase}-{round_index:03d}"
    argv = [
        str(runner), "run", "--manifest", str(frozen_panel), "--cast", str(binaries[label]),
        "--build-manifest", str(manifests[label]), "--rounds", "1",
        "--warmup-rounds", "1" if warmup else "0", "--round-offset", str(round_index),
        "--timeout-seconds", str(timeout), "--output-dir", str(output),
    ]
    if warmup:
        argv.append("--warmup-only")
    if rpc_env:
        argv.extend(["--rpc-env", rpc_env])
    if include_miss == "1":
        argv.append("--include-miss")
    schedule["execution_order"].append({"ref": label, "round": round_index, "phase": phase})
    write_json(root / "schedule.json", schedule)
    run(argv, repo)


for warmup, count in ((True, warmups), (False, rounds)):
    for round_index in range(count):
        labels = ("base", "candidate") if round_index % 2 == 0 else ("candidate", "base")
        for label in labels:
            execute(label, round_index, warmup)

# Preserve raw runs; aggregate references and records without copying child outputs.
for label in refs:
    aggregate = root / "results" / label / "aggregate"
    aggregate.mkdir()
    manifests_by_round = []
    with (aggregate / "samples.jsonl").open("w") as samples_out, (aggregate / "rpc-events.jsonl").open("w") as events_out:
        for round_index in range(rounds):
            measured = root / "results" / label / f"measured-{round_index:03d}"
            run_manifest = json.loads((measured / "manifest.json").read_text())
            manifests_by_round.append({
                "directory": str(measured),
                "manifest_sha256": digest(measured / "manifest.json"),
                "samples_sha256": digest(measured / "samples.jsonl"),
                "manifest": run_manifest,
            })
            namespace = measured.name
            for line in (measured / "samples.jsonl").read_text().splitlines():
                sample = json.loads(line)
                sample["source_sample_id"] = sample["id"]
                sample["source_run_directory"] = str(measured)
                sample["id"] = f"{namespace}/{sample['id']}"
                samples_out.write(json.dumps(sample) + "\n")
            events_path = measured / "rpc-events.jsonl"
            if events_path.exists():
                for line in events_path.read_text().splitlines():
                    event = json.loads(line)
                    event["source_sample_id"] = event["sample_id"]
                    event["source_run_directory"] = str(measured)
                    event["sample_id"] = f"{namespace}/{event['sample_id']}"
                    events_out.write(json.dumps(event) + "\n")
    write_json(aggregate / "manifest.json", {
        "schema_version": 1,
        "build": json.loads(manifests[label].read_text()),
        "panel": json.loads(frozen_panel.read_text()),
        "panel_sha256": digest(frozen_panel),
        "schedule": str(root / "schedule.json"),
        "rounds": rounds,
        "round_offset": 0,
        "warmup_only": False,
        "source_runs": manifests_by_round,
    })
    run([str(runner), "report", "--output-dir", str(aggregate)], repo)
print(f"Benchmark results and source worktrees retained in {root}")
PY
