#!/bin/bash
set -euo pipefail

# Script to commit benchmark results and read them for GitHub Actions output
# Usage: ./commit-and-read-benchmarks.sh <output_dir> <github_event_name> <github_repository>

OUTPUT_DIR="${1:-benches}"
GITHUB_EVENT_NAME="${2:-pull_request}"
GITHUB_REPOSITORY="${3:-}"

# Global variable for branch name
BRANCH_NAME=""

# Function to commit benchmark results
commit_results() {
    echo "Configuring git..."
    git config --local user.email "action@github.com"
    git config --local user.name "GitHub Action"

    # For PR runs, fetch and checkout the PR branch to ensure we're up to date
    if [ "$GITHUB_EVENT_NAME" = "pull_request" ] && [ -n "${GITHUB_HEAD_REF:-}" ]; then
        echo "Fetching latest changes for PR branch: $GITHUB_HEAD_REF"
        git fetch origin "$GITHUB_HEAD_REF"
        git checkout -B "$GITHUB_HEAD_REF" "origin/$GITHUB_HEAD_REF"
    fi

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
        if [ "$GITHUB_EVENT_NAME" = "workflow_dispatch" ]; then
            # For manual runs, we're on a new branch
            git push origin "$BRANCH_NAME"
        elif [ "$GITHUB_EVENT_NAME" = "pull_request" ]; then
            # For PR runs, push to the PR branch
            if [ -n "${GITHUB_HEAD_REF:-}" ]; then
                echo "Pushing to PR branch: $GITHUB_HEAD_REF"
                git push origin "$GITHUB_HEAD_REF"
            else
                echo "Error: GITHUB_HEAD_REF not set for pull_request event"
                exit 1
            fi
        else
            # This workflow should only run on workflow_dispatch or pull_request
            echo "Error: Unexpected event type: $GITHUB_EVENT_NAME"
            echo "This workflow only supports 'workflow_dispatch' and 'pull_request' events"
            exit 1
        fi
        echo "Successfully pushed benchmark results"
    fi
}

# Function to read benchmark results and output for GitHub Actions
read_results() {
    if [ -f "$OUTPUT_DIR/LATEST.md" ]; then
        echo "Reading benchmark results..."
        
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
}

# Main execution
echo "Starting benchmark results processing..."

# Create new branch for manual runs
if [ "$GITHUB_EVENT_NAME" = "workflow_dispatch" ]; then
    echo "Manual workflow run detected, creating new branch..."
    BRANCH_NAME="benchmarks/results-$(date +%Y%m%d-%H%M%S)"
    git checkout -b "$BRANCH_NAME"
    echo "Created branch: $BRANCH_NAME"
    
    # Output branch name for later use
    echo "branch_name=$BRANCH_NAME" >> "$GITHUB_OUTPUT"
fi

# Always commit benchmark results
echo "Committing benchmark results..."
commit_results

# Always read results for output
read_results

echo "Benchmark results processing complete"