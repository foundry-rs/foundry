#!/bin/bash
set -euo pipefail

# Bump all dependencies from the tempoxyz/tempo repository to the latest commit on main.
# All tempo dependencies share the same commit revision.
#
# Usage:
#   ./bump-tempo.sh [--dry-run]
#
# Requirements:
#   - gh (GitHub CLI) must be installed and authenticated
#
# Outputs (for GitHub Actions):
#   Sets outputs in $GITHUB_OUTPUT if it exists:
#     - current_rev: The current tempo revision
#     - latest_rev: The latest tempo revision on main
#     - updated: "true" if dependencies were updated, "false" otherwise
#     - changelog: Path to changelog file (if updated)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CARGO_TOML="$REPO_ROOT/Cargo.toml"

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=true
  echo "Running in dry-run mode (no changes will be made)"
fi

# Get the current tempo revision from any dependency using tempoxyz/tempo
# They all share the same rev, so we just grab the first one
get_current_rev() {
  grep 'git = "https://github.com/tempoxyz/tempo"' "$CARGO_TOML" | head -1 | sed 's/.*rev = "\([^"]*\)".*/\1/'
}

# Get the latest commit SHA from tempoxyz/tempo main branch
get_latest_rev() {
  gh api repos/tempoxyz/tempo/commits/main --jq '.sha'
}

# Generate changelog between two revisions
generate_changelog() {
  local old_rev="$1"
  local new_rev="$2"
  local output_file="$3"

  echo "Fetching commits from ${old_rev:0:7} to ${new_rev:0:7}..."

  local commits
  # shellcheck disable=SC2016 # Single quotes intentional for jq expression
  commits=$(gh api "repos/tempoxyz/tempo/compare/${old_rev}...${new_rev}" \
    --jq '.commits[] | "- [`\(.sha[0:7])`](https://github.com/tempoxyz/tempo/commit/\(.sha)) \(.commit.message | split("\n")[0])"')

  {
    echo "## Tempo Dependency Updates"
    echo ""
    echo "Bumped all \`tempo*\` dependencies from [\`${old_rev:0:7}\`](https://github.com/tempoxyz/tempo/commit/${old_rev}) to [\`${new_rev:0:7}\`](https://github.com/tempoxyz/tempo/commit/${new_rev})."
    echo ""
    echo "### Commits"
    echo ""
    echo "$commits"
  } > "$output_file"

  echo "Changelog written to $output_file"
}

# Update Cargo.toml with new revision
update_cargo_toml() {
  local old_rev="$1"
  local new_rev="$2"

  sed -i "s|git = \"https://github.com/tempoxyz/tempo\", rev = \"$old_rev\"|git = \"https://github.com/tempoxyz/tempo\", rev = \"$new_rev\"|g" "$CARGO_TOML"
  echo "Updated Cargo.toml: $old_rev -> $new_rev"
}

# Regenerate Cargo.lock (may fail if dependencies don't build)
regenerate_lockfile() {
  echo ""
  echo "Regenerating Cargo.lock..."
  if cargo generate-lockfile 2>&1; then
    echo "Cargo.lock regenerated successfully"
    set_output "lockfile_updated" "true"
  else
    echo "WARNING: Failed to regenerate Cargo.lock (dependencies may not build)"
    set_output "lockfile_updated" "false"
  fi
}

# Set GitHub Actions output if running in CI
set_output() {
  local name="$1"
  local value="$2"

  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    echo "$name=$value" >> "$GITHUB_OUTPUT"
  fi
}

main() {
  echo "=== Bump Tempo Dependencies ==="
  echo ""

  CURRENT_REV=$(get_current_rev)
  echo "Current revision: $CURRENT_REV"
  set_output "current_rev" "$CURRENT_REV"

  LATEST_REV=$(get_latest_rev)
  echo "Latest revision:  $LATEST_REV"
  set_output "latest_rev" "$LATEST_REV"

  if [[ "$CURRENT_REV" == "$LATEST_REV" ]]; then
    echo ""
    echo "Already up to date. No changes needed."
    set_output "updated" "false"
    exit 0
  fi

  echo ""
  echo "Update available: ${CURRENT_REV:0:7} -> ${LATEST_REV:0:7}"

  CHANGELOG_FILE="$REPO_ROOT/tempo-changelog.md"

  echo ""
  generate_changelog "$CURRENT_REV" "$LATEST_REV" "$CHANGELOG_FILE"
  set_output "changelog" "$CHANGELOG_FILE"

  if [[ "$DRY_RUN" == "true" ]]; then
    echo ""
    echo "[DRY RUN] Would update Cargo.toml"
    echo "[DRY RUN] Would regenerate Cargo.lock"
    echo ""
    echo "=== Changelog Preview ==="
    cat "$CHANGELOG_FILE"
  else
    update_cargo_toml "$CURRENT_REV" "$LATEST_REV"
    regenerate_lockfile
  fi

  set_output "updated" "true"

  echo ""
  echo "=== Done ==="
}

main "$@"
