#!/usr/bin/env python3
"""Compare nightly benchmark summaries using calibrated per-key thresholds."""

import argparse
import json
import math
import pathlib
import sys


def fail(message):
    print(f"Comparator error: {message}", file=sys.stderr)
    raise SystemExit(2)


def load_object(path, name):
    try:
        value = json.loads(pathlib.Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read {name}: {error}")
    if not isinstance(value, dict):
        fail(f"{name} must be a JSON object")
    return value


def positive_number(value, name):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
        or value <= 0
    ):
        fail(f"{name} must be finite and positive")
    return float(value)


def mean_of(value, key):
    if isinstance(value, dict):
        if "mean" not in value:
            fail(f"{key}: result object has no mean")
        value = value["mean"]
    return positive_number(value, f"{key} mean")


def load_rules(path):
    config = load_object(path, "threshold config")
    if (
        config.get("schema_version") != 1
        or config.get("kind") != "foundry-nightly-benchmark-thresholds"
    ):
        fail("threshold config has unsupported schema_version or kind")
    rules = config.get("benchmarks")
    if not isinstance(rules, dict):
        fail("threshold config benchmarks must be an object")
    for key, rule in rules.items():
        if not isinstance(key, str) or not isinstance(rule, dict):
            fail("each threshold rule must be a keyed object")
        warning = positive_number(rule.get("warning_percent"), f"{key} warning_percent")
        regression = positive_number(
            rule.get("regression_percent"), f"{key} regression_percent"
        )
        if warning > regression or not isinstance(rule.get("alert"), bool):
            fail(f"{key}: invalid threshold ordering or alert value")
    return rules


def duration(seconds):
    if seconds < 0.001:
        return f"{seconds * 1000:.2f}ms"
    if seconds < 1:
        return f"{seconds:.3f}s"
    if seconds < 60:
        return f"{seconds:.2f}s"
    return f"{int(seconds // 60)}m {seconds % 60:.1f}s"


def compare(base, candidate, rules):
    print("## Nightly Benchmark Regression Report\n")
    print("| Benchmark | Stable | Nightly | Change |")
    print("|-----------|--------:|---------:|--------|")
    has_regression = False
    for key in sorted(base.keys() | candidate.keys()):
        has_base, has_candidate = key in base, key in candidate
        if not has_base or not has_candidate:
            present = duration(mean_of(candidate[key] if not has_base else base[key], key))
            left, right = ("N/A", present) if not has_base else (present, "N/A")
            print(f"| `{key}` | {left} | {right} | ⚠️ Inconclusive (missing side) |")
            continue

        baseline = mean_of(base[key], key)
        nightly = mean_of(candidate[key], key)
        delta = (nightly - baseline) / baseline * 100
        rule = rules.get(key)
        if rule is None:
            verdict = "⚪ Uncalibrated"
        else:
            warning = rule["warning_percent"]
            regression = rule["regression_percent"]
            alert = rule["alert"]
            suffix = "" if alert else " (advisory)"
            if delta >= regression:
                verdict = "❌ Regression" + suffix
                has_regression |= alert
            elif delta >= warning:
                verdict = "⚠️ Warning" + suffix
            elif delta <= -warning:
                verdict = "✅ Improvement" + suffix
            else:
                verdict = "⚪ Within threshold" + suffix
        print(
            f"| `{key}` | {duration(baseline)} | {duration(nightly)} | "
            f"{delta:+.2f}% {verdict} |"
        )

    print(
        "\nLegend: ❌ regression, ⚠️ warning/inconclusive, ✅ improvement, "
        "⚪ within threshold or uncalibrated. Advisory results never alert."
    )
    return 1 if has_regression else 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("base")
    parser.add_argument("candidate")
    parser.add_argument("thresholds")
    args = parser.parse_args()
    base_path = pathlib.Path(args.base)
    base = load_object(base_path, "baseline") if base_path.is_file() else {}
    candidate = load_object(args.candidate, "candidate")
    rules = load_rules(args.thresholds)
    return compare(base, candidate, rules)


if __name__ == "__main__":
    sys.exit(main())
