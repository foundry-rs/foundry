#!/usr/bin/env bash
set -euo pipefail

# Keeps Foundry's Tempo crates decoupled from mpp-rs and foundry-wallets.
#
# Foundry takes tempo-* crates from tempoxyz/tempo by git revision. mpp-rs keeps its own crates.io
# tempo-* release, and foundry-wallets has no Tempo dependency, so Tempo releases and API changes
# never have to compile through either crate. This fails if:
#   - foundry-wallets depends on a tempo-* crate;
#   - mpp-rs, or a crates.io tempo-* crate it pulls, takes a tempo-* crate from anywhere but
#     crates.io, for example through a [patch.crates-io] entry that forces tempo-alloy or
#     tempo-primitives onto Foundry's Tempo revision;
#   - anything other than mpp-rs, or crates.io tempo-* crates themselves, depends on crates.io
#     tempo-* crates.
#
# Usage:
#   ./.github/scripts/check-tempo-deps.sh

violations="$(
  cargo metadata --locked --format-version 1 --all-features |
    jq -r '
      (.packages | map({key: .id, value: .}) | from_entries) as $pkgs
      | .resolve.nodes[] as $node
      | $pkgs[$node.id] as $parent
      | $node.deps[]
      | $pkgs[.pkg] as $dep
      | select($dep.name | startswith("tempo-"))
      | (($dep.source // "") | startswith("registry+")) as $dep_from_registry
      | (($parent.source // "") | startswith("registry+")) as $parent_from_registry
      | select(
          $parent.name == "foundry-wallets"
          or ((($parent.name | IN("mpp", "alloy-transport-mpp"))
               or (($parent.name | startswith("tempo-")) and $parent_from_registry))
              and ($dep_from_registry | not))
          or ($dep_from_registry
              and ($parent.name | IN("mpp", "alloy-transport-mpp") | not)
              and (($parent.name | startswith("tempo-")) and $parent_from_registry | not))
        )
      | "\($parent.name) \($parent.version) -> \($dep.name) \($dep.version) (\($dep.source // "path"))"
    ' |
    sort -u
)"

if [[ -n "$violations" ]]; then
  echo "ERROR: Tempo crates are coupled across mpp-rs or foundry-wallets:" >&2
  echo "$violations" >&2
  echo "Foundry's tempo-* crates must not reach foundry-wallets or replace mpp-rs's crates.io release." >&2
  exit 1
fi

echo "Tempo dependencies are decoupled."
