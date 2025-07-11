#!/bin/bash
set -euo pipefail

# Script to commit benchmark results and read them for GitHub Actions output
# Usage: ./commit-and-read-benchmarks.sh <output_dir> <github_event_name> <github_repository>

OUTPUT_DIR="${1:-benches}"
GITHUB_EVENT_NAME="${2:-pull_request}"
GITHUB_REPOSITORY="${3:-}"

# Function to commit benchmark results
commit_results() {
    echo "Configuring git..."
    git config --local user.email "action@github.com"
    git config --local user.name "GitHub Action"

    echo "Adding benchmark file..."
    git add "$OUTPUT_DIR/LATEST.md"

    if git diff --staged --quiet; then
        echo "No changes to commit"
    else
        echo "Committing benchmark results..."
        git commit -m "chore(\`benches\`): update benchmark results

🤖 Generated with [Foundry Benchmarks](https://github.com/${GITHUB_REPOSITORY}/actions)

Co-Authored-By: github-actions <github-actions@github.com>"
        
        echo "Pushing to repository..."
        git push
        echo "Successfully pushed benchmark results"
    fi
}

# Function to read benchmark results and output for GitHub Actions
read_results() {
    if [ -f "$OUTPUT_DIR/LATEST.md" ]; then
        echo "Reading benchmark results..."
        
        # Extract Forge Test results summary
        echo "Extracting Forge Test summary..."
        {
            echo 'forge_test_summary<<EOF'
            # Extract the Forge Test section from the markdown
            awk '/^## Forge Test Results/,/^## Forge Build.*Results/' "$OUTPUT_DIR/LATEST.md" | head -n -1
            echo 'EOF'
        } >> "$GITHUB_OUTPUT"
        
        # Output full results
        {
            echo 'results<<EOF'
            cat "$OUTPUT_DIR/LATEST.md"
            echo 'EOF'
        } >> "$GITHUB_OUTPUT"
        echo "Successfully read benchmark results"
    else
        echo 'results=No benchmark results found.' >> "$GITHUB_OUTPUT"
        echo 'forge_test_summary=No Forge Test results found.' >> "$GITHUB_OUTPUT"
        echo "Warning: No benchmark results found at $OUTPUT_DIR/LATEST.md"
    fi
}

# Main execution
echo "Starting benchmark results processing..."

# Only commit if not a pull request
if [ "$GITHUB_EVENT_NAME" != "pull_request" ]; then
    echo "Event is not a pull request, proceeding with commit..."
    commit_results
else
    echo "Event is a pull request, skipping commit"
fi

# Always read results for output
read_results

echo "Benchmark results processing complete"