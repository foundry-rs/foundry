#!/usr/bin/env python3
"""Render a Cast BAL campaign as one self-contained GitHub Markdown comment."""

import argparse
from collections import Counter
import json
import math
from pathlib import Path
import re
from statistics import median


MARKER = "<!-- foundry-cast-bal-benchmark -->"
ARMS = {"auto": "BAL-accelerated", "replay": "Full replay"}
REFS = {"base": "Base", "candidate": "PR"}
MAX_FILE_BYTES = 16 * 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_text(root, relative):
    path = root / relative
    require(path.resolve().is_relative_to(root.resolve()), "Evidence path escapes campaign directory")
    require(path.is_file(), f"Evidence file missing: {relative}")
    require(path.stat().st_size <= MAX_FILE_BYTES, f"Evidence file too large: {relative}")
    return path.read_text()


def read_json(root, relative):
    return json.loads(read_text(root, relative))


def safe_text(value):
    text = " ".join(str(value).split())[:500]
    for char, replacement in (("&", "&amp;"), ("<", "&lt;"), (">", "&gt;"),
                              ("@", "&#64;"), ("|", "&#124;"), ("`", "&#96;"),
                              ("[", "&#91;"), ("]", "&#93;"), ("*", "&#42;"),
                              ("_", "&#95;"), ("\\", "&#92;")):
        text = text.replace(char, replacement)
    return text


def is_hash(value, length):
    return isinstance(value, str) and re.fullmatch(rf"[0-9a-fA-F]{{{length}}}", value) is not None


def integer(value, minimum=0):
    return type(value) is int and minimum <= value


def duration(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def quantile(values, fraction):
    values = sorted(values)
    position = (len(values) - 1) * fraction
    lower, upper = math.floor(position), math.ceil(position)
    return values[lower] + (values[upper] - values[lower]) * (position - lower)


def rpc_count(sample, direction="client"):
    counters = sample["rpc"][f"{direction}_requests_by_method"]
    require(isinstance(counters, dict) and all(integer(x) for x in counters.values()),
            "Invalid RPC request counters")
    return sum(counters.values())


def validate_samples(samples, cases, rounds, warmups):
    expected = Counter((case["id"], phase, arm, index)
                       for case in cases
                       for phase, count in (("validation", 1), ("warmup", warmups), ("measured", rounds))
                       for arm in ARMS for index in range(count))
    expected.update((case["id"], "oracle", "replay", 0) for case in cases)
    actual = Counter((sample["case_id"], sample["phase"], sample["arm"], sample["round"])
                     for sample in samples)
    require(actual == expected, "Missing, duplicate or unexpected scheduled attempts")
    require(len({sample["id"] for sample in samples}) == len(samples), "Duplicate sample identifiers")
    by_case = {case["id"]: case for case in cases}
    for sample in samples:
        case = by_case[sample["case_id"]]
        require(sample["schema_version"] == 1 and integer(sample["round"]), "Invalid sample schema")
        require(sample["timed_out"] is False and type(sample["exit_code"]) is int
                and sample["exit_code"] == 0, "Failed or timed-out attempt")
        require(duration(sample["wall_time_seconds"]) and sample["wall_time_seconds"] > 0
                and duration(sample["observed_duration_seconds"]),
                "Invalid sample duration")
        expected_path = "bal_hit" if sample["arm"] == "auto" else "replay_no_probe"
        require(sample["actual_path"] == expected_path, "Unexpected execution path")
        require(sample["correctness"] == ("unchecked" if sample["phase"] == "oracle" else "equivalent")
                and sample["local_gas"] == case["expected_receipt_gas"]
                and sample["execution_success"] is case["expected_receipt_status"]
                and sample["fault"] is None and sample["synthetic"] is False,
                "Receipt or equivalence check failed")
        require(is_hash(sample["stdout_sha256"], 64), "Invalid output digest")
        rpc_count(sample)
        rpc_count(sample, "upstream")
        require(integer(sample["rpc"]["client_response_body_bytes"]), "Invalid RPC byte counter")
    for case in cases:
        selected = [sample for sample in samples if sample["case_id"] == case["id"]]
        validation = [sample for sample in selected if sample["phase"] == "validation"]
        require(len({sample["stdout_sha256"] for sample in validation}) == 1,
                "BAL and full replay validation traces differ")
        ordinary = [sample for sample in selected if sample["phase"] != "validation"]
        require(len({sample["stdout_sha256"] for sample in ordinary}) == 1,
                "Measured output differs from the replay oracle")


def validate_run(run, summary, panel, block, cases, label):
    config = summary["configuration"]
    require(all(is_hash(summary["files"][name]["sha256"], 64)
                for name in ("base_cast", "head_cast")), "Invalid campaign binary identity")
    require(run["schema_version"] == 1, "Unsupported native run schema")
    require(all(run[key] == config[key] for key in
                ("rounds", "warmup_rounds", "timeout_seconds", "worker_count")),
            "Native run configuration differs from campaign")
    require(run["include_miss"] is False, "Unexpected injected miss arm")
    require(run["panel"]["cases"] == cases and run["panel"]["blocks"] == [block]
            and run["panel"]["chain_id"] == panel["chain_id"], "Native run panel differs from campaign")
    binary = summary["files"]["base_cast" if label == "base" else "head_cast"]["sha256"]
    require(is_hash(binary, 64) and run["binary"]["sha256"] == binary,
            "Native Cast binary identity differs from campaign")
    if run.get("build") is not None:
        ref = summary["refs"]["base" if label == "base" else "head"]
        require(run["build"]["source_sha"] == ref and run["build"]["cast"]["sha256"] == binary,
                "Native build provenance differs from pinned revision")


def collect(root, summary):
    """Retain diagnostics on failed blocks, but validate every success before comparing."""
    config = summary["configuration"]
    require(integer(config["rounds"], 1) and config["rounds"] <= 1000
            and integer(config["warmup_rounds"]) and config["warmup_rounds"] <= 1000
            and integer(config["timeout_seconds"], 1) and config["worker_count"] == 1,
            "Invalid campaign configuration")
    panel = read_json(root, "manifest.json")
    require(panel["schema_version"] == 1 and 0 < len(panel["cases"]) <= 32
            and 0 < len(panel["blocks"]) <= 16, "Invalid campaign panel")
    case_ids = [case["id"] for case in panel["cases"]]
    require(len(set(case_ids)) == len(case_ids) and all(
        isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,80}", value) for value in case_ids),
        "Invalid or duplicate case identifier")
    numbers = [block["block_number"] for block in panel["blocks"]]
    require(len(set(numbers)) == len(numbers) and all(integer(number, 1) for number in numbers),
            "Invalid or duplicate block number")
    require(len({block["block_hash"] for block in panel["blocks"]}) == len(numbers),
            "Duplicate block hash")
    require(all(case["block_hash"] in {block["block_hash"] for block in panel["blocks"]}
                and integer(case["expected_receipt_gas"])
                and type(case["expected_receipt_status"]) is bool
                and integer(case["index"]) for case in panel["cases"]), "Invalid case metadata")
    blocks = summary["blocks"]
    require(isinstance(blocks, list) and len(blocks) <= len(numbers)
            and len({block["block_number"] for block in blocks}) == len(blocks)
            and all(block["block_number"] in numbers for block in blocks), "Invalid campaign block list")
    info = {block["block_number"]: block for block in blocks}
    rows, issues = [], []
    for block in panel["blocks"]:
        number = block["block_number"]
        cases = [case for case in panel["cases"] if case["block_hash"] == block["block_hash"]]
        state = info.get(number, {})
        if state.get("status") != "pass":
            issues.append(f"Block {number}: {state.get('error', 'not completed')}")
        else:
            require(state["fixture_barrier_closed"] is True and state["barrier_attempts"] == 0
                    and state["fixture_failures"] == 0 and state["verification"]["issues"] == []
                    and state["verification"]["cross_ref_receipt_gas_and_status_match"] is True,
                    f"Block {number}: parent-state or receipt verification failed")
        for label in REFS:
            relative = f"block-{number}/results/{label}/aggregate"
            samples = []
            try:
                samples = [json.loads(line) for line in read_text(root, f"{relative}/samples.jsonl").splitlines()
                           if line.strip()]
                require(len(samples) <= 150000, "Too many native samples")
                run = read_json(root, f"{relative}/manifest.json")
                validate_run(run, summary, panel, block, cases, label)
                validate_samples(samples, cases, config["rounds"], config["warmup_rounds"])
            except (OSError, ValueError, KeyError, TypeError) as error:
                issues.append(f"Block {number}, {REFS[label]}: {error}")
            for case in cases:
                for arm in ARMS:
                    selected = [sample for sample in samples if isinstance(sample, dict)
                                and sample.get("case_id") == case["id"]
                                and sample.get("arm") == arm and sample.get("phase") == "measured"]
                    rows.append({"case": case, "block": number, "ref": label, "arm": arm,
                                 "samples": selected})
    return rows, issues


def measurement_details(rows, complete):
    lines = ["<details>", "<summary>Per-case measurements and correctness</summary>", ""]
    for case_id in dict.fromkeys(row["case"]["id"] for row in rows):
        selected = [row for row in rows if row["case"]["id"] == case_id]
        case = selected[0]["case"]
        lines += [f"**{case_id}** — block {selected[0]['block']}, transaction index {case['index']}.", "",
                  f"Receipt: {case['expected_receipt_gas']:,} gas; "
                  f"{'success' if case['expected_receipt_status'] else 'reverted'}. "
                  f"Transaction: `{safe_text(case.get('transaction_hash', 'unknown'))}`.", "",
                  "| Revision / mode | Attempts / timeouts / failed exits | Wall ms: median / IQR / min / max | Local RPCs: client / forwarded | Response KiB |",
                  "| --- | ---: | ---: | ---: | ---: |"]
        for row in selected:
            samples = row["samples"]
            counts = f"{len(samples)} / {sum(sample.get('timed_out') is True for sample in samples)} / "
            counts += str(sum(sample.get("exit_code") != 0 for sample in samples))
            wall, rpc, size = "withheld", "withheld", "withheld"
            if complete:
                values = [sample["wall_time_seconds"] * 1000 for sample in samples]
                wall = " / ".join(f"{value:.3f}" for value in (
                    median(values), quantile(values, .75) - quantile(values, .25), min(values), max(values)))
                rpc = f"{median(rpc_count(sample) for sample in samples):g} / "
                rpc += f"{median(rpc_count(sample, 'upstream') for sample in samples):g}"
                size = f"{median(sample['rpc']['client_response_body_bytes'] for sample in samples) / 1024:.1f}"
            lines.append(f"| {REFS[row['ref']]} / {ARMS[row['arm']]} | {counts} | {wall} | {rpc} | {size} |")
        lines.append("")
    lines += ["Wall statistics use measured rounds only; IQR = p75 − p25 with linear interpolation. "
              "RPC and response-byte columns are per-attempt medians after proxy cleanup. "
              "Forwarded RPCs go to the local gateway, not an archive provider. "
              "Timeouts are censored and are never substituted as elapsed measurements.", "", "</details>", ""]
    return lines


def render(root, base_sha=None, head_sha=None, run_url=None, failure=None):
    summary, rows, issues = {}, [], []
    try:
        summary = read_json(root, "campaign.json")
        require(summary["schema_version"] == 1, "Unsupported campaign schema")
        refs = summary["refs"]
        require(is_hash(refs["base"], 40) and is_hash(refs["head"], 40), "Invalid campaign revisions")
        require((base_sha is None or refs["base"] == base_sha)
                and (head_sha is None or refs["head"] == head_sha),
                "Campaign revisions differ from the requested PR base/head")
        base_sha, head_sha = refs["base"], refs["head"]
        if summary["status"] != "complete":
            issues.append(f"Campaign {summary['status']}: {summary.get('error', 'did not complete')}")
        rows, collected = collect(root, summary)
        issues.extend(collected)
    except (OSError, ValueError, KeyError, TypeError) as error:
        issues.append(f"Evidence unavailable or invalid: {error}")
    if failure:
        issues.insert(0, failure)
    control = bool(rows) and (base_sha == head_sha or summary["files"]["base_cast"]["sha256"]
                             == summary["files"]["head_cast"]["sha256"])
    validated = bool(rows) and not issues
    complete = validated and not control
    lines = [MARKER, "## Cast BAL benchmark", ""]
    if base_sha and head_sha:
        lines += [f"Base `{safe_text(base_sha)}` → PR `{safe_text(head_sha)}`.", ""]
    if run_url:
        require(re.fullmatch(r"https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/actions/runs/[0-9]+"
                             r"(?:/attempts/[1-9][0-9]*)?", run_url),
                "Invalid GitHub Actions run URL")
        lines += [f"[Benchmark run]({run_url})", ""]
    lines += ["Frozen historical cases on warmed local Anvil with authentic parent state and captured BALs. "
              "Preparation and warmup are excluded; local BAL transfer and processing are included. "
              "This measures local execution, not public-RPC latency or network-wide performance.", ""]
    if validated and control:
        config = summary["configuration"]
        measured = sum(len(row["samples"]) for row in rows)
        lines += [f"**Validated control:** {len(rows) // 4} cases, {config['rounds']} measured rounds "
                  f"per revision/mode, {config['warmup_rounds']} warmup rounds, "
                  f"{config['timeout_seconds']} s timeout, one worker. "
                  f"All {measured}/{len(rows) * config['rounds']} measured attempts passed. "
                  "The campaign completed and passed all receipt, replay-equivalence and preparation-barrier checks.", "",
                  "This control uses the same revision or binary for Base and PR. It verifies benchmark execution; "
                  "it is not PR performance evidence. PR timing comparisons and ratios are withheld.", ""]
    elif complete:
        config = summary["configuration"]
        lines += [f"**Complete:** {len(rows) // 4} cases, {config['rounds']} measured rounds per revision/mode, "
                  f"{config['warmup_rounds']} warmup rounds, {config['timeout_seconds']} s timeout, one worker. "
                  "Both revisions match receipt gas/status and their own full replay traces; "
                  "all scheduled attempts passed, with no fixture access after preparation.", "",
                  "Each row compares **Base → PR in the same mode**. Ratio = PR / Base median "
                  "(below 1 is faster); no overall speedup or statistical significance is inferred.", "",
                  "| Case | Mode | Median ms, Base → PR | Time ratio | Local RPCs, Base → PR | RPC ratio |",
                  "| --- | --- | ---: | ---: | ---: | ---: |"]
        for row in rows:
            if row["ref"] != "base":
                continue
            head = next(item for item in rows if item["case"]["id"] == row["case"]["id"]
                        and item["arm"] == row["arm"] and item["ref"] == "candidate")
            base_time, head_time = [median(sample["wall_time_seconds"] for sample in item["samples"])
                                    for item in (row, head)]
            base_rpc, head_rpc = [median(rpc_count(sample) for sample in item["samples"])
                                 for item in (row, head)]
            time_ratio = f"{head_time / base_time:.3f}×" if base_time else "n/a"
            rpc_ratio = f"{head_rpc / base_rpc:.3f}×" if base_rpc else "n/a"
            lines.append(f"| {row['case']['id']} | {ARMS[row['arm']]} | "
                         f"{base_time * 1000:.3f} → {head_time * 1000:.3f} | {time_ratio} | "
                         f"{base_rpc:g} → {head_rpc:g} | {rpc_ratio} |")
        lines += ["", "Local RPCs count JSON-RPC calls from Cast to the local proxy, including metadata "
                  "and optional probes. BAL-accelerated requires an observed BAL hit; Full replay adds "
                  "`--no-bal` to the same revision's binary.", ""]
    else:
        lines += ["**No valid PR performance comparison.** Timings and ratios are withheld because "
                  "the complete campaign did not pass all checks.", ""]
        lines.extend(f"- {safe_text(issue)}" for issue in issues[:20])
        if control:
            lines.append("- This control uses the same revision or binary; it is not PR performance evidence.")
        if len(issues) > 20:
            lines.append(f"- {len(issues) - 20} additional evidence errors.")
        lines.append("")
    if rows:
        lines.extend(measurement_details(rows, complete))
    return "\n".join(lines).rstrip() + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campaign-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-sha")
    parser.add_argument("--head-sha")
    parser.add_argument("--run-url")
    parser.add_argument("--failure")
    args = parser.parse_args(argv)
    if any(value is not None and not is_hash(value, 40) for value in (args.base_sha, args.head_sha)):
        parser.error("base-sha and head-sha must be full Git commit hashes")
    report = render(args.campaign_dir, args.base_sha, args.head_sha, args.run_url, args.failure)
    require(len(report.encode()) < 60000, "Report exceeds GitHub comment size budget")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
