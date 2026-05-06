#!/bin/bash
set -euo pipefail

# Script to read benchmark results and emit them as GitHub Actions outputs.
# This script performs no git operations — it only reads the combined
# benchmark file and writes outputs for the workflow to consume.
#
# Usage: ./read-benchmark-results.sh <output_dir>

OUTPUT_DIR="${1:-benches}"

echo "Reading benchmark results from $OUTPUT_DIR..."

if [ -f "$OUTPUT_DIR/LATEST.md" ]; then
    # Output full results
    {
        echo 'results<<EOF'
        cat "$OUTPUT_DIR/LATEST.md"
        echo 'EOF'
    } >> "$GITHUB_OUTPUT"

    # Format results for PR comment
    echo "Formatting results for PR comment..."
    FORMATTED_COMMENT=$("$(dirname "$0")/format-pr-comment.sh" "$OUTPUT_DIR/LATEST.md")

    {
        echo 'pr_comment<<EOF'
        echo "$FORMATTED_COMMENT"
        echo 'EOF'
    } >> "$GITHUB_OUTPUT"

    echo "Successfully read and formatted benchmark results"
else
    echo 'results=No benchmark results found.' >> "$GITHUB_OUTPUT"
    echo 'pr_comment=No benchmark results found.' >> "$GITHUB_OUTPUT"
    echo "Warning: No benchmark results found at $OUTPUT_DIR/LATEST.md"
fi
