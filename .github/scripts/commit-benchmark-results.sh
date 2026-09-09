#!/bin/bash
set -euo pipefail

# Script to commit and push benchmark results.
#
# This script is intended to run from the lightweight `publish-results` job,
# which checks out the repo with credentials and only operates on the
# trusted artifact produced by the benchmark job. Keeping the write-scoped
# token away from the bench job (which runs untrusted third-party builds)
# limits the blast radius of a compromised dependency.
#
# Usage: ./commit-benchmark-results.sh <output_dir> <github_event_name> <github_repository>

OUTPUT_DIR="${1:-benches}"
GITHUB_EVENT_NAME="${2:-workflow_dispatch}"
GITHUB_REPOSITORY="${3:-}"

if [ ! -f "$OUTPUT_DIR/LATEST.md" ]; then
    echo "Error: $OUTPUT_DIR/LATEST.md not found, nothing to commit"
    exit 1
fi

echo "Configuring git..."
git config --local user.email "action@github.com"
git config --local user.name "GitHub Action"

# Decide which branch to commit to based on the event.
BRANCH_NAME=""
case "$GITHUB_EVENT_NAME" in
    workflow_dispatch)
        echo "Manual workflow run detected, creating new branch..."
        BRANCH_NAME="benchmarks/results-$(date +%Y%m%d-%H%M%S)"
        git checkout -b "$BRANCH_NAME"
        echo "Created branch: $BRANCH_NAME"
        ;;
    pull_request)
        if [ -z "${GITHUB_HEAD_REF:-}" ]; then
            echo "Error: GITHUB_HEAD_REF not set for pull_request event"
            exit 1
        fi
        echo "Fetching latest changes for PR branch: $GITHUB_HEAD_REF"
        git fetch origin "$GITHUB_HEAD_REF"
        git checkout -B "$GITHUB_HEAD_REF" "origin/$GITHUB_HEAD_REF"
        BRANCH_NAME="$GITHUB_HEAD_REF"
        ;;
    *)
        echo "Error: Unexpected event type: $GITHUB_EVENT_NAME"
        echo "This workflow only supports 'workflow_dispatch' and 'pull_request' events"
        exit 1
        ;;
esac

# Always emit the branch name so downstream steps (e.g. PR creation) can use it.
echo "branch_name=$BRANCH_NAME" >> "$GITHUB_OUTPUT"

echo "Adding benchmark file..."
git add "$OUTPUT_DIR/LATEST.md"

if git diff --staged --quiet; then
    echo "No changes to commit"
    echo "committed=false" >> "$GITHUB_OUTPUT"
    exit 0
fi

echo "Committing benchmark results..."
git commit -m "chore(\`benches\`): update benchmark results

🤖 Generated with [Foundry Benchmarks](https://github.com/${GITHUB_REPOSITORY}/actions)

Co-Authored-By: github-actions <github-actions@github.com>"

echo "Pushing to repository..."
git push origin "$BRANCH_NAME"
echo "Successfully pushed benchmark results to $BRANCH_NAME"
echo "committed=true" >> "$GITHUB_OUTPUT"
