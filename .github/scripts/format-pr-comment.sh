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

# Find where "## Forge Build" starts and split the content
# Extract everything before "## Forge Build"
BEFORE_FORGE_BUILD=$(echo "$CONTENT" | awk '/^## Forge Build$/ {exit} {print}')

# Extract everything from "## Forge Build" onwards
FROM_FORGE_BUILD=$(echo "$CONTENT" | awk '/^## Forge Build$/ {found=1} found {print}')

# Output the formatted comment with dropdown
cat << EOF
${BEFORE_FORGE_BUILD}

<details>
<summary>📈 View all benchmark results</summary>

${FROM_FORGE_BUILD}

</details>
EOF