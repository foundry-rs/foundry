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

# Count distinct "## Forge " sections to decide whether to collapse.
# Fuzz-only reports have a single compact table — no need for a dropdown.
FORGE_SECTION_COUNT=$(echo "$CONTENT" | grep -c '^## Forge ' || true)

if [ "$FORGE_SECTION_COUNT" -le 1 ]; then
    # Compact report: render inline, no dropdown.
    cat << EOF
${BEFORE_TABLES}

${FROM_TABLES}
EOF
else
    # Multiple sections (perf or combined): collapse into a dropdown.
    cat << EOF
${BEFORE_TABLES}

<details>
<summary>📈 View all benchmark results</summary>

${FROM_TABLES}

</details>
EOF
fi