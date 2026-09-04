#!/usr/bin/env bash

set -euo pipefail

workspace=${1:-.}
cooldown_days=${CARGO_COOLDOWN_DAYS:-7}
version=0.3.4

if [[ ! "$cooldown_days" =~ ^[1-9][0-9]*$ ]]; then
  echo "ERROR: CARGO_COOLDOWN_DAYS must be a positive whole number" >&2
  exit 1
fi

workspace=$(cd "$workspace" && pwd)
lockfile="$workspace/Cargo.lock"
if [[ ! -f "$lockfile" ]] || ! git -C "$workspace" ls-files --error-unmatch Cargo.lock >/dev/null 2>&1; then
  echo "ERROR: Cargo.lock must exist and be committed in $workspace" >&2
  exit 1
fi
if ! git -C "$workspace" diff --quiet HEAD -- Cargo.lock; then
  echo "ERROR: Cargo.lock must match the committed version in $workspace" >&2
  exit 1
fi

config="$workspace/cooldown.toml"
if [[ -f "$config" ]]; then
  if ! git -C "$workspace" ls-files --error-unmatch cooldown.toml >/dev/null 2>&1 ||
    ! git -C "$workspace" diff --quiet HEAD -- cooldown.toml; then
    echo "ERROR: cooldown.toml must match the committed version in $workspace" >&2
    exit 1
  fi
  python3 -I - "$config" <<'PY'
import sys
import tomllib
from pathlib import Path

path = Path(sys.argv[1])
data = tomllib.loads(path.read_text())
if set(data) != {"allow"} or set(data["allow"]) != {"exact"}:
    raise SystemExit(f"ERROR: {path} may contain only [[allow.exact]] rules")
for rule in data["allow"]["exact"]:
    if set(rule) != {"crate", "version"} or not all(
        isinstance(rule[key], str) and rule[key] for key in ("crate", "version")
    ):
        raise SystemExit(f"ERROR: invalid [[allow.exact]] rule in {path}")
PY
fi

runner_os=${RUNNER_OS:-}
runner_arch=${RUNNER_ARCH:-}
if [[ -z "$runner_os" ]]; then
  case $(uname -s) in
    Linux) runner_os=Linux ;;
    Darwin) runner_os=macOS ;;
    *) runner_os=unsupported ;;
  esac
fi
if [[ -z "$runner_arch" ]]; then
  case $(uname -m) in
    x86_64) runner_arch=X64 ;;
    arm64 | aarch64) runner_arch=ARM64 ;;
    *) runner_arch=unsupported ;;
  esac
fi

case "$runner_os/$runner_arch" in
  Linux/X64)
    target=x86_64-unknown-linux-gnu
    expected=5706c1636415b90ec8244631ff707d1c49d502779d14235d7abb65c080ac8ba6
    ;;
  Linux/ARM64)
    target=aarch64-unknown-linux-gnu
    expected=bc5488d892da21575f1834ad31c64077f2a681ea70d22e4409fa3f21857cf63a
    ;;
  macOS/X64)
    target=x86_64-apple-darwin
    expected=fd7e8627a8248f3e803a866a893cd3ec2db29c215dee16357caa4d23a36c86f0
    ;;
  macOS/ARM64)
    target=aarch64-apple-darwin
    expected=0499a6ff956cd1bfb4b29466cbbf1ceb4571a1d1c0eabfe84bffa487e2b11d4b
    ;;
  *)
    echo "ERROR: unsupported cargo-cooldown runner $runner_os/$runner_arch" >&2
    exit 1
    ;;
esac

runner_temp=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
install_dir=$(mktemp -d "$runner_temp/cargo-cooldown-install.XXXXXX")
cargo_home=$(mktemp -d "$runner_temp/cargo-cooldown-home.XXXXXX")
lockfile_snapshot=$(mktemp "$runner_temp/cargo-cooldown-lock.XXXXXX")
cp "$lockfile" "$lockfile_snapshot"

cleanup() {
  if ! cmp -s "$lockfile_snapshot" "$lockfile"; then
    cp "$lockfile_snapshot" "$lockfile"
  fi
  rm -rf "$install_dir" "$cargo_home" "$lockfile_snapshot"
}
trap cleanup EXIT

root="cargo-cooldown-${target}-v${version}"
archive="$install_dir/${root}.tgz"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location --retry 3 \
  --output "$archive" \
  "https://github.com/dertin/cargo-cooldown/releases/download/v${version}/${root}.tgz"
if [[ "$runner_os" == macOS ]] && command -v shasum >/dev/null 2>&1; then
  printf '%s  %s\n' "$expected" "$archive" | shasum -a 256 --check
elif command -v sha256sum >/dev/null 2>&1; then
  printf '%s  %s\n' "$expected" "$archive" | sha256sum --check --strict
elif command -v shasum >/dev/null 2>&1; then
  printf '%s  %s\n' "$expected" "$archive" | shasum -a 256 --check
else
  echo "ERROR: sha256sum or shasum is required" >&2
  exit 1
fi
tar -xzf "$archive" -C "$install_dir"
cooldown_bin="$install_dir/$root/cargo-cooldown"
if [[ ! -x "$cooldown_bin" ]]; then
  echo "ERROR: verified cargo-cooldown archive did not contain $cooldown_bin" >&2
  exit 1
fi

while IFS='=' read -r name _; do
  case "$name" in
    COOLDOWN_* | CARGO_REGISTRY_MIN_PUBLISH_AGE | CARGO_REGISTRY_*_MIN_PUBLISH_AGE | CARGO_REGISTRIES_*_MIN_PUBLISH_AGE)
      unset "$name"
      ;;
  esac
done < <(env)

export CARGO_HOME="$cargo_home"
export CARGO_REGISTRY_GLOBAL_MIN_PUBLISH_AGE="$cooldown_days days"
export COOLDOWN_INCOMPATIBLE_PUBLISH_AGE=deny
export COOLDOWN_LOCKFILE_BASELINE=ignore
export RUSTC_WRAPPER=""

set +e
(
  cd "$workspace"
  "$cooldown_bin" --workspace --all-features tree --locked --target all --depth 0 >/dev/null
)
status=$?
set -e

if ! cmp -s "$lockfile_snapshot" "$lockfile"; then
  echo "ERROR: cargo-cooldown changed $lockfile; refusing the unchecked graph" >&2
  exit 1
fi
if [[ "$status" -ne 0 ]]; then
  echo "ERROR: Cargo dependency cooldown failed in $workspace" >&2
  exit "$status"
fi
