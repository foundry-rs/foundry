#!/bin/sh -ex

cargo test
cargo test --no-default-features

cargo test --features slice16-mem-limit
cargo test --features bytewise-mem-limit
cargo test --features no-table-mem-limit

cargo test --features bytewise-mem-limit,slice16-mem-limit
cargo test --features no-table-mem-limit,bytewise-mem-limit
cargo test --features no-table-mem-limit,slice16-mem-limit

cargo test --features no-table-mem-limit,bytewise-mem-limit,slice16-mem-limit

