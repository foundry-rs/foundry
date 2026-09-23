#!/usr/bin/env bash
set -u
scripts_dir=$(cd "$(dirname "$0")" && pwd)
trap 'exit 0' TERM INT
while true; do
  node "$scripts_dir/macos-network-probe.cjs"
  [[ "${1:-}" != '--once' ]] || break
  sleep 60 &
  wait "$!" || break
done
