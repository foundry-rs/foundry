#!/usr/bin/env bash
# Compare two benchmark JSON summaries and report regressions/improvements.
#
# Usage: compare-nightly.sh <base.json> <candidate.json> [warn_pct] [fail_pct]
#
# Wall time is lower-is-better. A change is judged against a noise band equal to
# the combined relative stddev of both sides (scaled by NOISE_MULT), so a delta
# only counts when it exceeds the threshold beyond noise. Single-run metrics
# (no stddev, e.g. coverage) are reported but not judged.
#
# Env overrides (defaults preserve the nightly regression report):
#   BASE_LABEL          baseline column label     (default: Stable)
#   CANDIDATE_LABEL     candidate column label     (default: Nightly)
#   REPORT_TITLE        heading                    (default: ## Nightly Benchmark Regression Report)
#   NOISE_MULT          noise band multiplier      (default: 1.0)
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
REPORT_TITLE="${REPORT_TITLE:-## Nightly Benchmark Regression Report}" \
NOISE_MULT="${NOISE_MULT:-1.0}" FAIL_ON_REGRESSION="${FAIL_ON_REGRESSION:-1}" \
python3 - <<'EOF'
import json, math, os, sys

warn = float(os.environ["WARN"])
fail = float(os.environ["FAIL"])
noise_mult = float(os.environ["NOISE_MULT"])
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

def stddev_of(v):
    return v.get("stddev") if isinstance(v, dict) else None

print(os.environ["REPORT_TITLE"] + "\n")
print(f"| Benchmark | {base_label} | {cand_label} | Δ | Status |")
print("|-----------|--------|---------|---|--------|")

has_regression = False
for key in sorted(base.keys() | cand.keys()):
    b = base.get(key)
    c = cand.get(key)
    if c is None:
        print(f"| `{key}` | {mean_of(b):.5f}s | N/A | — | ⚠️ Missing |")
        has_regression = True
        continue
    if b is None:
        print(f"| `{key}` | N/A | {mean_of(c):.5f}s | — | 🆕 New |")
        continue
    bm, cm = mean_of(b), mean_of(c)
    bs, cs = stddev_of(b), stddev_of(c)
    delta = (cm - bm) / bm * 100
    sign = "+" if delta > 0 else ""
    if bs is None or cs is None:
        status = "⚪ Not judged"
    else:
        band = math.hypot(bs / bm, cs / cm) * 100 * noise_mult
        if delta - band >= fail:
            status = "🔴 Regression"
            has_regression = True
        elif delta - band >= warn:
            status = "🟡 Warning"
        elif -delta - band >= warn:
            status = "🟢 Improvement"
        else:
            status = "➡️ OK"
    print(f"| `{key}` | {bm}s | {cm}s | {sign}{delta:.1f}% | {status} |")

sys.exit(1 if has_regression and fail_on_regression else 0)
EOF
