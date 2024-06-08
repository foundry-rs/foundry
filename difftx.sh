#!/usr/bin/env bash
set -eo pipefail

cargo build --bin cast

echo "-- Running cast with interpreter"
./target/debug/cast run "$@" &> interpreter.log

echo "-- Running cast with JIT"
JIT="" ./target/debug/cast run "$@" &> jit.log

difft interpreter.log jit.log
