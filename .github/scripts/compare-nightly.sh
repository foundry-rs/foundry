#!/usr/bin/env bash
# Compare two nightly benchmark JSON summaries and report regressions.
#
# Usage: compare-nightly.sh <prev.json> <today.json> [warn_pct] [fail_pct]
# Exits 0 if no regressions, 1 if any metric exceeds fail_pct.
# Exits 0 gracefully when prev.json is missing (first run / gap > 7 days).
set -euo pipefail

PREV_JSON="${1:-}"
TODAY_JSON="${2:-}"
WARN="${3:-1}"
FAIL="${4:-3}"

PREV_JSON="$PREV_JSON" TODAY_JSON="$TODAY_JSON" WARN="$WARN" FAIL="$FAIL" \
python3 - <<'EOF'
import json, os, sys

warn = float(os.environ["WARN"])
fail = float(os.environ["FAIL"])

prev_path = os.environ.get("PREV_JSON", "")
prev = json.load(open(prev_path)) if prev_path and os.path.isfile(prev_path) else {}
with open(os.environ["TODAY_JSON"]) as f:
    today = json.load(f)

print("## Nightly Benchmark Regression Report\n")
print("| Benchmark | Stable | Nightly | Δ | Status |")
print("|-----------|--------|---------|---|--------|")

has_regression = False
all_keys = sorted(prev.keys() | today.keys())
for key in all_keys:
    t = today.get(key)
    p = prev.get(key)
    if t is None:
        print(f"| `{key}` | {p:.5f}s | N/A | — | ⚠️ Missing |")
        has_regression = True
        continue
    if p is None:
        print(f"| `{key}` | N/A | {t:.5f}s | — | 🆕 New |")
        continue
    delta = (t - p) / p * 100
    if delta >= fail:
        status = "🔴 Regression"
        has_regression = True
    elif delta >= warn:
        status = "🟡 Warning"
    elif delta <= -warn:
        status = "🟢 Improvement"
    else:
        status = "➡️ OK"
    sign = "+" if delta > 0 else ""
    print(f"| `{key}` | {p}s | {t}s | {sign}{delta:.1f}% | {status} |")

sys.exit(1 if has_regression else 0)
EOF
