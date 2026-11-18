#!/bin/bash
set -euo pipefail

# Bump all dependencies from the tempoxyz/tempo repository to the latest commit on main.
# All tempo dependencies share the same commit revision.
# The evm2 and reth revisions follow Tempo's new revision when Tempo's are newer.
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
#     - latest_rev_short: The short latest tempo revision
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

# Print the revision of the first dependency in a manifest that uses a git URL.
get_rev() {
  local manifest="$1"
  local url="$2"
  { grep -F "git = \"$url\"" "$manifest" || true; } | head -1 | sed 's/.*rev = "\([^"]*\)".*/\1/'
}

# Get the current tempo revision from any dependency using tempoxyz/tempo
# They all share the same rev, so we just grab the first one
get_current_rev() {
  get_rev "$CARGO_TOML" "https://github.com/tempoxyz/tempo"
}

# Get the latest commit SHA from tempoxyz/tempo main branch
get_latest_rev() {
  gh api repos/tempoxyz/tempo/commits/main --jq '.sha'
}

# Generate changelog between two revisions.
# Use redirect.github.com links to avoid backlinks on upstream PR timelines.
generate_changelog() {
  local old_rev="$1"
  local new_rev="$2"
  local output_file="$3"

  echo "Fetching commits from ${old_rev:0:7} to ${new_rev:0:7}..."

  local commits
  # shellcheck disable=SC2016 # Single quotes intentional for jq expression
  commits=$(gh api "repos/tempoxyz/tempo/compare/${old_rev}...${new_rev}" \
    --jq '.commits[] | "- [`\(.sha[0:7])`](https://redirect.github.com/tempoxyz/tempo/commit/\(.sha)) \(.commit.message | split("\n")[0] | gsub("#(?<number>[0-9]+)"; "[#\(.number)](https://redirect.github.com/tempoxyz/tempo/issues/\(.number))"))"')

  {
    echo "## Tempo Dependency Updates"
    echo ""
    echo "Bumped all \`tempo*\` dependencies from [\`${old_rev:0:7}\`](https://redirect.github.com/tempoxyz/tempo/commit/${old_rev}) to [\`${new_rev:0:7}\`](https://redirect.github.com/tempoxyz/tempo/commit/${new_rev})."
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

# Move a pin that Foundry shares with Tempo to Tempo's revision, but only forward.
# Both manifests must pin the same git URL; GitHub's compare API decides which revision is newer.
sync_pin() {
  local name="$1"
  local repo="$2"
  local url="$3"
  local tempo_manifest="$4"
  local changelog="$5"
  local old_rev new_rev status

  old_rev=$(get_rev "$CARGO_TOML" "$url")
  new_rev=$(get_rev "$tempo_manifest" "$url")
  if [[ -z "$old_rev" || -z "$new_rev" ]]; then
    echo "WARNING: $url is not pinned in both manifests; not syncing $name"
    return
  fi
  if [[ "$old_rev" == "$new_rev" ]]; then
    return
  fi

  status=$(gh api "repos/$repo/compare/${old_rev}...${new_rev}" --jq '.status')
  if [[ "$status" != "ahead" ]]; then
    echo "Keeping $name at ${old_rev:0:7}: Tempo's ${new_rev:0:7} is not newer ($status)"
    echo "- Kept \`$name\` at [\`${old_rev:0:7}\`](https://redirect.github.com/$repo/commit/${old_rev}); Tempo's [\`${new_rev:0:7}\`](https://redirect.github.com/$repo/commit/${new_rev}) is not newer (compare status: $status)." >> "$changelog"
    return
  fi

  if [[ "$DRY_RUN" != "true" ]]; then
    sed -i.bak "s/$old_rev/$new_rev/g; s/\`${old_rev:0:7}\`/\`${new_rev:0:7}\`/g" "$CARGO_TOML"
    rm "$CARGO_TOML.bak"
  fi
  echo "Synced $name with Tempo: ${old_rev:0:7} -> ${new_rev:0:7}"
  echo "- Bumped \`$name\` to Tempo's revision: [\`${old_rev:0:7}...${new_rev:0:7}\`](https://redirect.github.com/$repo/compare/${old_rev}...${new_rev})." >> "$changelog"
}

# Sync the evm2 and reth revisions that Tempo's new revision builds against.
sync_tempo_pins() {
  local new_rev="$1"
  local changelog="$2"
  local tempo_manifest notes
  tempo_manifest=$(mktemp)
  notes=$(mktemp)

  gh api "repos/tempoxyz/tempo/contents/Cargo.toml?ref=${new_rev}" -H "Accept: application/vnd.github.raw" > "$tempo_manifest"
  # Both manifests force the whole graph onto one evm2 revision through this patch URL.
  sync_pin "evm2" "alloy-rs/evm2" "https://github.com/DaniPopes/evm2" "$tempo_manifest" "$notes"
  sync_pin "reth" "paradigmxyz/reth" "https://github.com/paradigmxyz/reth" "$tempo_manifest" "$notes"
  if [[ -s "$notes" ]]; then
    {
      echo ""
      echo "### Synced with Tempo"
      echo ""
      cat "$notes"
    } >> "$changelog"
  fi
  rm "$tempo_manifest" "$notes"
}

# Resolve the new revision while preserving unrelated locked dependencies.
regenerate_lockfile() {
  echo ""
  echo "Updating Cargo.lock..."
  if cargo update --workspace 2>&1; then
    echo "Cargo.lock updated successfully"
    set_output "lockfile_updated" "true"
  else
    echo "ERROR: Failed to update Cargo.lock"
    set_output "lockfile_updated" "false"
    return 1
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
  set_output "latest_rev_short" "${LATEST_REV:0:7}"

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
    echo "[DRY RUN] Would update Cargo.lock, preserving unrelated dependencies"
    sync_tempo_pins "$LATEST_REV" "$CHANGELOG_FILE"
    echo ""
    echo "=== Changelog Preview ==="
    cat "$CHANGELOG_FILE"
  else
    update_cargo_toml "$CURRENT_REV" "$LATEST_REV"
    sync_tempo_pins "$LATEST_REV" "$CHANGELOG_FILE"
    regenerate_lockfile
  fi

  set_output "updated" "true"

  echo ""
  echo "=== Done ==="
}

main "$@"
