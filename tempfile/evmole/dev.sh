#!/bin/bash

MODE=$1
DS=$2

BDIR=`pwd`/benchmark

ln -s `pwd` ${BDIR}/providers/evmole-rs/rust 2>/dev/null || true

case ${NOTRACE+x} in
    x) FEAT='' ;;
    *) FEAT="--features evmole/trace_${MODE}" ;;
esac

cargo run \
    --manifest-path benchmark/providers/evmole-rs/Cargo.toml \
    ${FEAT} \
    ${MODE} \
    ${BDIR}/datasets/${2} \
    out.json \
    ${BDIR}/results/etherscan.selectors_${2}.json \
    --filter-filename ${3} \
    --filter-selector ${4}

rm -rf ${BDIR}/providers/evmole-rs/rust
