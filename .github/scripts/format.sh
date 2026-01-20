#!/usr/bin/env bash
set -eo pipefail

# We have to ignore at shell level because testdata/ is not a valid Foundry project,
# so running `forge fmt` with `--root testdata` won't actually check anything
sol_files=()
while IFS= read -r -d '' file; do
    sol_files+=("$file")
done < <(find testdata -name '*.sol' ! -name Vm.sol ! -name console.sol -print0)

# Run forge fmt on all found files
cargo run --bin forge -- fmt "$@" "${sol_files[@]}"
