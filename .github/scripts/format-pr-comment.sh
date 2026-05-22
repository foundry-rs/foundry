#!/bin/bash
set -euo pipefail

# Script to format benchmark results for PR comment
# Usage: ./format-pr-comment.sh <benchmark_results_file>

RESULTS_FILE="${1:-}"

if [ -z "$RESULTS_FILE" ] || [ ! -f "$RESULTS_FILE" ]; then
    echo "Error: Benchmark results file not provided or does not exist"
    exit 1
fi

# Read the file content
CONTENT=$(cat "$RESULTS_FILE")

# Split at the first benchmark result section (## Forge …) so the summary
# header stays visible in the comment body and all tables go in the dropdown.
# This works for both perf-only, fuzz-only, and combined LATEST.md files.
BEFORE_TABLES=$(echo "$CONTENT" | awk '/^## Forge / {exit} {print}')
FROM_TABLES=$(echo "$CONTENT" | awk '/^## Forge / {found=1} found {print}')

# Output the formatted comment with dropdown
cat << EOF
${BEFORE_TABLES}

<details>
<summary>📈 View all benchmark results</summary>

${FROM_TABLES}

</details>
EOF