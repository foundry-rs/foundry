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
REFS = {"base": "Base", "candidate": "Head"}
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


def median_interval(values, groups=2):
    """Return an exact median interval with at least 95% joint coverage across groups."""
    values = sorted(values)
    count, tail, interval = len(values), 0, None
    for rank in range(1, (count + 1) // 2 + 1):
        tail += math.comb(count, rank - 1)
        # Bonferroni: two tails per group share a 0.05 error budget.
        if 40 * groups * tail > 2 ** count:
            break
        interval = values[rank - 1], values[count - rank]
    return interval


def wall_assessment(base, head):
    intervals = [median_interval(values) for values in (base, head)]
    if any(interval is None for interval in intervals):
        return "Inconclusive", "Too few measured rounds (at least 7 per revision required)."
    before, after = intervals
    evidence = (f"Median intervals: Base {before[0] * 1000:.3f}–{before[1] * 1000:.3f} ms; "
                f"Head {after[0] * 1000:.3f}–{after[1] * 1000:.3f} ms.")
    if after[1] < before[0]:
        return "Improved", f"{evidence} Head interval is entirely lower."
    if after[0] > before[1]:
        return "Regressed", f"{evidence} Head interval is entirely higher."
    return "Inconclusive", f"{evidence} Intervals overlap or touch; direction is unresolved."


def percent_change(before, after):
    if before == after:
        return "unchanged"
    if before == 0:
        return f"{before:g} → {after:g}"
    percent = abs(after / before - 1) * 100
    amount = "<0.1%" if percent < .05 else f"{percent:.1f}%"
    return f"{amount} {'lower' if after < before else 'higher'}"


def comparisons(rows):
    for base in rows:
        if base["ref"] == "base":
            head = next(row for row in rows if row["ref"] == "candidate"
                        and row["case"]["id"] == base["case"]["id"] and row["arm"] == base["arm"])
            yield base, head


def case_modes(rows, case_id):
    return {label: {row["arm"]: [sample["wall_time_seconds"] for sample in row["samples"]]
                    for row in rows if row["case"]["id"] == case_id and row["ref"] == label}
            for label in REFS}


def speedup(modes):
    return median(modes["replay"]) / median(modes["auto"])


def speedup_assessment(modes):
    intervals = []
    for label in REFS:
        bal, replay = [median_interval(modes[label][arm], groups=4) for arm in ARMS]
        if bal is None or replay is None:
            return "Inconclusive", "Too few measured rounds (at least 8 per revision/mode required)."
        intervals.append((replay[0] / bal[1], replay[1] / bal[0]))
    before, after = intervals
    evidence = (f"Speedup intervals: Base {before[0]:.3f}–{before[1]:.3f}×; "
                f"Head {after[0]:.3f}–{after[1]:.3f}×.")
    if after[0] > before[1]:
        return "Improved", f"{evidence} Head has a larger BAL benefit relative to its own full replay."
    if after[1] < before[0]:
        return "Regressed", f"{evidence} Head has a smaller BAL benefit relative to its own full replay."
    return "Inconclusive", f"{evidence} Intervals overlap or touch; direction is unresolved."


def resource_changes(base, head):
    changes = []
    for name, metric in (("RPC", rpc_count),
                         ("Response", lambda sample: sample["rpc"]["client_response_body_bytes"])):
        values = [median(metric(sample) for sample in row["samples"]) for row in (base, head)]
        if values[0] != values[1]:
            change = percent_change(*values)
            if name == "Response" and values[0] == 0:
                change += " bytes"
            changes.append(f"{name}: {change}")
    return "; ".join(changes) or "RPC / response unchanged"


def measurement_summary(rows, control):
    if control:
        lines = ["| Case | Base speedup | Head speedup |", "| --- | ---: | ---: |"]
    else:
        lines = ["| Case | Base speedup | Head speedup | Speedup change | BAL benefit |",
                 "| --- | ---: | ---: | ---: | --- |"]
    for case_id in dict.fromkeys(row["case"]["id"] for row in rows):
        modes = case_modes(rows, case_id)
        before, after = [speedup(modes[label]) for label in REFS]
        line = f"| {case_id} | {before:.2f}× | {after:.2f}× |"
        if not control:
            status, _ = speedup_assessment(modes)
            line += f" {percent_change(before, after)} | {status} |"
        lines.append(line)
    lines += ["", "Speedup = Full replay wall ms / BAL wall ms, using measured-round medians. "
              "Higher is better; 1× means equal time, and below 1× means BAL is slower. "
              "Warmup is excluded. Expand the details for absolute wall times, resource changes "
              "and assessment reasons.", ""]
    return lines


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


def measurement_details(rows, validated, control):
    lines = ["<details>", "<summary>Per-case measurements and correctness</summary>", ""]
    if validated and not control:
        lines += ["BAL-benefit assessments use four exact, nonparametric median intervals, one per "
                  "revision/mode, each with at least 98.75% coverage (at least 95% joint coverage per case). "
                  "Each speedup interval is [replay lower / BAL upper, replay upper / BAL lower]. "
                  "Improved requires the Head speedup interval to be entirely higher than Base; Regressed "
                  "requires it to be entirely lower. Overlapping or touching intervals, or fewer than "
                  "8 measured rounds per revision/mode, give Inconclusive. This does not establish equal benefit.", "",
                  "Absolute wall-time assessments use 97.5% median intervals for each revision "
                  "(at least 95% joint coverage), with at least 7 measured rounds. Lower is faster. "
                  "These exploratory assessments assume representative independent "
                  "repetitions; they do not provide simultaneous confidence across all cases or rule out "
                  "machine drift. RPC and response changes describe observed medians, not statistical "
                  "verdicts or a combined performance score.", ""]
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
            if validated:
                values = [sample["wall_time_seconds"] * 1000 for sample in samples]
                wall = " / ".join(f"{value:.3f}" for value in (
                    median(values), quantile(values, .75) - quantile(values, .25), min(values), max(values)))
                rpc = f"{median(rpc_count(sample) for sample in samples):g} / "
                rpc += f"{median(rpc_count(sample, 'upstream') for sample in samples):g}"
                size = f"{median(sample['rpc']['client_response_body_bytes'] for sample in samples) / 1024:.1f}"
            lines.append(f"| {REFS[row['ref']]} / {ARMS[row['arm']]} | {counts} | {wall} | {rpc} | {size} |")
        lines.append("")
        if validated and not control:
            status, reason = speedup_assessment(case_modes(selected, case_id))
            lines += [f"**BAL benefit: {status}.** {reason}", ""]
            for base, head in comparisons(selected):
                times = [[sample["wall_time_seconds"] for sample in row["samples"]]
                         for row in (base, head)]
                status, reason = wall_assessment(*times)
                change = percent_change(*map(median, times))
                lines += [f"**{ARMS[base['arm']]} wall time: {status} ({change}).** {reason} "
                          f"{resource_changes(base, head)}.", ""]
    lines += ["BAL-accelerated requires an observed BAL hit; Full replay adds `--no-bal` to the "
              "same revision's binary. Local RPCs count JSON-RPC calls from Cast to the local proxy, "
              "including metadata and optional probes.", "",
              "Wall statistics use measured rounds only; IQR = p75 − p25 with linear interpolation. "
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
        lines += [f"Base `{safe_text(base_sha)}` → Head `{safe_text(head_sha)}`.", ""]
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
                  "This control uses the same revision or binary; it is not PR performance evidence. "
                  "Base and Head each show BAL speedup against "
                  "their own full replay; no change between revisions is assessed.", ""]
    elif complete:
        config = summary["configuration"]
        lines += [f"**Complete:** {len(rows) // 4} cases, {config['rounds']} measured rounds per revision/mode, "
                  f"{config['warmup_rounds']} warmup rounds, {config['timeout_seconds']} s timeout, one worker. "
                  "Both revisions match receipt gas/status and their own full replay traces; "
                  "all scheduled attempts passed, with no fixture access after preparation.", "",
                  "Each row compares **BAL's advantage over Full replay in Base versus Head**. "
                  "Speedup change = (Head speedup / Base speedup − 1) × 100%. "
                  "This measures relative benefit, not absolute BAL latency. Full replay is measured "
                  "on both revisions; a larger speedup can also result from slower replay. "
                  "Inconclusive means the direction is unresolved; expand details for separate "
                  "wall-time and resource changes.", ""]
    else:
        lines += ["**No valid PR performance comparison.** Timings and assessments are withheld because "
                  "the complete campaign did not pass all checks.", ""]
        lines.extend(f"- {safe_text(issue)}" for issue in issues[:20])
        if control:
            lines.append("- This control uses the same revision or binary; it is not PR performance evidence.")
        if len(issues) > 20:
            lines.append(f"- {len(issues) - 20} additional evidence errors.")
        lines.append("")
    if validated:
        lines.extend(measurement_summary(rows, control))
    if rows:
        lines.extend(measurement_details(rows, validated, control))
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
