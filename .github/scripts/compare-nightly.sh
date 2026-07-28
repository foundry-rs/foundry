#!/usr/bin/env bash
# Compare two benchmark JSON summaries and report regressions/improvements.
#
# Usage: compare-nightly.sh <base.json> <candidate.json> [warn_pct] [fail_pct]
#
# Wall time is lower-is-better. Thresholds apply to the raw percentage change,
# preserving the existing nightly alert contract.
#
# Env overrides (defaults preserve the nightly regression report):
#   BASE_LABEL          baseline column label     (default: Stable)
#   CANDIDATE_LABEL     candidate column label     (default: Nightly)
#   REPORT_TITLE        heading                    (default: ## Nightly Benchmark Regression Report)
#   FAIL_ON_REGRESSION  exit 1 on any regression   (default: 1)
#
# Exits 0 if no regressions (or FAIL_ON_REGRESSION=0), 1 otherwise.
# Exits 0 gracefully when base.json is missing (first run / gap > 7 days).
set -euo pipefail

BASE_JSON="${1:-}"
CAND_JSON="${2:-}"
WARN="${3:-1}"
FAIL="${4:-3}"

BASE_JSON="$BASE_JSON" CAND_JSON="$CAND_JSON" WARN="$WARN" FAIL="$FAIL" \
BASE_LABEL="${BASE_LABEL:-Stable}" CANDIDATE_LABEL="${CANDIDATE_LABEL:-Nightly}" \
REPORT_TITLE="${REPORT_TITLE-## Nightly Benchmark Regression Report}" \
FAIL_ON_REGRESSION="${FAIL_ON_REGRESSION:-1}" \
python3 - <<'EOF'
import json, os, sys

warn = float(os.environ["WARN"])
fail = float(os.environ["FAIL"])
fail_on_regression = os.environ["FAIL_ON_REGRESSION"] != "0"
base_label = os.environ["BASE_LABEL"]
cand_label = os.environ["CANDIDATE_LABEL"]

base_path = os.environ.get("BASE_JSON", "")
base = json.load(open(base_path)) if base_path and os.path.isfile(base_path) else {}
with open(os.environ["CAND_JSON"]) as f:
    cand = json.load(f)

# Each value is either the current summary object ({"mean": ..., "stddev": ...})
# or, for historical files, a bare mean-seconds float.
def mean_of(v):
    return v["mean"] if isinstance(v, dict) else v

def fmt_duration(seconds):
    if seconds < 0.001:
        return f"{seconds * 1000:.2f}ms"
    if seconds < 1:
        return f"{seconds:.3f}s"
    if seconds < 60:
        return f"{seconds:.2f}s"
    minutes = int(seconds // 60)
    return f"{minutes}m {seconds % 60:.1f}s"

title = os.environ["REPORT_TITLE"]
if title:
    print(title + "\n")
print(f"| Benchmark | {base_label} | {cand_label} | Change |")
print("|-----------|--------:|---------:|--------|")

has_regression = False
for key in sorted(base.keys() | cand.keys()):
    b = base.get(key)
    c = cand.get(key)
    if c is None:
        print(f"| `{key}` | {fmt_duration(mean_of(b))} | N/A | ⚠️ Missing |")
        has_regression = True
        continue
    if b is None:
        print(f"| `{key}` | N/A | {fmt_duration(mean_of(c))} | 🆕 New |")
        continue
    bm, cm = mean_of(b), mean_of(c)
    delta = (cm - bm) / bm * 100
    sign = "+" if delta > 0 else ""
    if delta >= fail:
        verdict = "❌"
        has_regression = True
    elif delta >= warn:
        verdict = "⚠️"
    elif delta <= -warn:
        verdict = "✅"
    else:
        verdict = "⚪"
    change = f"{sign}{delta:.2f}% {verdict}"
    print(f"| `{key}` | {fmt_duration(bm)} | {fmt_duration(cm)} | {change} |")

sys.exit(1 if has_regression and fail_on_regression else 0)
EOF
