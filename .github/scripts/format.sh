#!/usr/bin/env bash
set -eo pipefail

# We have to ignore at shell level because testdata/ is not a valid Foundry project,
# so running `forge fmt` with `--root testdata` won't actually check anything
cargo run --bin forge -- fmt "$@" \
    $(find testdata -name '*.sol' ! -name Vm.sol ! -name console.sol)
