#!/usr/bin/bash

# fail fast
#
set -e

# print each command before it's executed
#
set -x

wget https://github.com/EmbarkStudios/cargo-deny/releases/download/0.8.2/cargo-deny-0.8.2-x86_64-unknown-linux-musl.tar.gz \
     -O - | tar -xz

cargo-deny-0.8.2-x86_64-unknown-linux-musl/cargo-deny check
