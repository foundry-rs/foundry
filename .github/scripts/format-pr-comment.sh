#!/bin/bash
set -euo pipefail

# Script to format benchmark results for PR comment
# Usage: ./format-pr-comment.sh <benchmark_results_file>

RESULTS_FILE="${1:-}"

if [ -z "$RESULTS_FILE" ] || [ ! -f "$RESULTS_FILE" ]; then
    echo "Error: Benchmark results file not provided or does not exist"
    exit 1
fi

# Keep the headline comparison compact; the generated absolute-time report is
# still available for auditing without dominating the PR conversation.
cat <<EOF
<details>
<summary>Full benchmark results</summary>

$(cat "$RESULTS_FILE")

</details>
EOF
