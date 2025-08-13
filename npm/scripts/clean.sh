#!/usr/bin/env bash

set -eou pipefail

rm -rf bin dist
rm -rf ./@foundry-rs/forge*/bin ./@foundry-rs/forge*/dist ./@foundry-rs/forge*/*.tgz