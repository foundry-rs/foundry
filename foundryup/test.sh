#!/usr/bin/env bash

# Tests for the foundryup migration bootstrap. `download` and the installer are
# mocked so no network calls are made.

# Mocks/globals are used indirectly by the sourced foundryup functions.
# shellcheck disable=SC2329,SC2034,SC2317

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Source foundryup for its function definitions without running the bootstrap.
export FOUNDRYUP_TEST=1
# shellcheck source=/dev/null
. "$SCRIPT_DIR/foundryup"

failures=0

check_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" = "$actual" ]; then
    printf 'ok   - %s\n' "$desc"
  else
    printf 'FAIL - %s\n      expected: %q\n      actual:   %q\n' "$desc" "$expected" "$actual"
    failures=$((failures + 1))
  fi
}

# Silence progress output; `err` keeps exiting via the overridden `say`.
say() { :; }
warn() { :; }

# `exec` replaces the process; stub it so we can observe the final hand-off
# instead of losing the test runner. Records args and exits the subshell.
exec_marker=""
exec() {
  echo "$*" >> "$exec_marker"
  exit 0
}

# Sets up an isolated FOUNDRY_DIR/bin for one test and re-points the shim globals
# at it. Leaves a sentinel launcher so we can assert it is or isn't replaced.
setup_case() {
  FOUNDRY_DIR="$(mktemp -d)"
  FOUNDRY_BIN_DIR="$FOUNDRY_DIR/bin"
  FOUNDRY_BIN_PATH="$FOUNDRY_BIN_DIR/foundryup"
  mkdir -p "$FOUNDRY_BIN_DIR"
  printf 'original-launcher\n' > "$FOUNDRY_BIN_PATH"
  exec_marker="$(mktemp)"
  : > "$exec_marker"
}

teardown_case() {
  rm -rf "$FOUNDRY_DIR"
  rm -f "$exec_marker"
}

# A `download` that writes a fake foundryup-init.sh installing a working fake
# binary into the staging FOUNDRY_DIR. `$fake_bin_body` controls the binary.
fake_bin_body='#!/usr/bin/env sh
echo "foundryup 0.0.5 (test 2020)"'
download() {
  cat > "$2" <<EOF
#!/usr/bin/env sh
echo "installer chatter that must not reach stdout"
mkdir -p "\$FOUNDRY_DIR/bin"
cat > "\$FOUNDRY_DIR/bin/foundryup" <<'BIN'
$fake_bin_body
BIN
chmod +x "\$FOUNDRY_DIR/bin/foundryup"
EOF
  return 0
}

# --- happy path -----------------------------------------------------------

setup_case
out="$( ( bootstrap --install stable 2>/dev/null ) )"
check_eq "bootstrap execs the installed Rust binary with original args" \
  "env FOUNDRYUP_BOOTSTRAP_ACTIVE=1 $FOUNDRY_BIN_PATH --install stable" \
  "$(cat "$exec_marker")"
check_eq "bootstrap replaces the launcher with the Rust binary" \
  "foundryup 0.0.5 (test 2020)" "$("$FOUNDRY_BIN_PATH")"
check_eq "bootstrap keeps stdout clean (installer chatter on stderr)" "" "$out"
teardown_case

# --- loop guard -----------------------------------------------------------

setup_case
rc=0; ( FOUNDRYUP_BOOTSTRAP_ACTIVE=1 bootstrap 2>/dev/null ) || rc=$?
check_eq "loop guard aborts a re-entrant bootstrap" "1" "$rc"
check_eq "loop guard exec not attempted" "" "$(cat "$exec_marker")"
teardown_case

# --- download failure -----------------------------------------------------

setup_case
download() { return 1; }
rc=0; ( bootstrap 2>/dev/null ) || rc=$?
check_eq "download failure aborts with nonzero exit" "1" "$rc"
check_eq "download failure leaves launcher untouched" \
  "original-launcher" "$(cat "$FOUNDRY_BIN_PATH")"
check_eq "download failure does not exec" "" "$(cat "$exec_marker")"
teardown_case

# --- non-script installer body (e.g. 404) ---------------------------------

setup_case
download() { printf '404: Not Found\n' > "$2"; return 0; }
rc=0; ( bootstrap 2>/dev/null ) || rc=$?
check_eq "non-script installer body is rejected" "1" "$rc"
check_eq "rejected body leaves launcher untouched" \
  "original-launcher" "$(cat "$FOUNDRY_BIN_PATH")"
teardown_case

# --- installer produces no binary -----------------------------------------

setup_case
download() { printf '#!/usr/bin/env sh\nexit 0\n' > "$2"; return 0; }
rc=0; ( bootstrap 2>/dev/null ) || rc=$?
check_eq "missing installed binary is rejected" "1" "$rc"
check_eq "missing binary leaves launcher untouched" \
  "original-launcher" "$(cat "$FOUNDRY_BIN_PATH")"
teardown_case

# --- staged binary fails validation ---------------------------------------

setup_case
download() {
  cat > "$2" <<'EOF'
#!/usr/bin/env sh
mkdir -p "$FOUNDRY_DIR/bin"
printf '#!/usr/bin/env sh\nexit 3\n' > "$FOUNDRY_DIR/bin/foundryup"
chmod +x "$FOUNDRY_DIR/bin/foundryup"
EOF
  return 0
}
rc=0; ( bootstrap 2>/dev/null ) || rc=$?
check_eq "binary failing --version is rejected" "1" "$rc"
check_eq "failed validation leaves launcher untouched" \
  "original-launcher" "$(cat "$FOUNDRY_BIN_PATH")"
check_eq "failed validation does not exec" "" "$(cat "$exec_marker")"
teardown_case

# --- summary --------------------------------------------------------------

if [ "$failures" -ne 0 ]; then
  printf '\n%d test(s) failed\n' "$failures"
  exit 1
fi
printf '\nall tests passed\n'
