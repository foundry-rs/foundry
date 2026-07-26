#!/usr/bin/env bash
# MPP (Machine Payments Protocol) end-to-end test script
#
# Prerequisites:
#   - A Tempo Accounts `store.json`, or `TEMPO_ROTATE_WALLET=1` with an isolated `TEMPO_HOME`
#   - Wallet funded with PathUSD on Moderato
#   - Foundry binaries built: `cargo build --bin cast --bin forge --bin anvil --bin chisel`
#
# Usage:
#   ./scripts/mpp-test.sh [binary-dir]
#
# Examples:
#   ./scripts/mpp-test.sh                         # uses cast/forge from PATH
#   ./scripts/mpp-test.sh ./target/debug          # use debug builds

set -euo pipefail

BIN_DIR="${1:-}"
if [ -n "$BIN_DIR" ]; then
  BIN_DIR="$(cd "$BIN_DIR" && pwd)"
  CAST="$BIN_DIR/cast"
  FORGE="$BIN_DIR/forge"
  ANVIL="$BIN_DIR/anvil"
  CHISEL="$BIN_DIR/chisel"
else
  CAST="cast"
  FORGE="forge"
  ANVIL="anvil"
  CHISEL="chisel"
fi
TEMPO_AUTO_FUND="${TEMPO_AUTO_FUND:-0}"
TEMPO_AUTO_FUND_ATTEMPTS="${TEMPO_AUTO_FUND_ATTEMPTS:-3}"
MIN_BALANCE="${MPP_MIN_BALANCE:-1000000}"
RPC_MPP="${MPP_RPC_URL:-https://rpc.mpp.moderato.tempo.xyz}"
RPC="${TEMPO_RPC_URL:-https://rpc.moderato.tempo.xyz}"
TOKEN="${MPP_TOKEN:-0x20c0000000000000000000000000000000000000}"  # PathUSD on Moderato

if ! command -v "$CAST" &>/dev/null; then
  echo "ERROR: cast binary not found at '$CAST'. Install with: foundryup"
  exit 1
fi
if ! command -v "$FORGE" &>/dev/null; then
  echo "ERROR: forge binary not found at '$FORGE'. Install with: foundryup"
  exit 1
fi
if ! command -v "$ANVIL" &>/dev/null; then
  echo "ERROR: anvil binary not found at '$ANVIL'. Install with: foundryup"
  exit 1
fi
if ! command -v "$CHISEL" &>/dev/null; then
  echo "ERROR: chisel binary not found at '$CHISEL'. Install with: foundryup"
  exit 1
fi
if ! command -v jq &>/dev/null; then
  echo "ERROR: jq is required"
  exit 1
fi

CHAIN_ID=$("$CAST" chain-id --rpc-url "$RPC")

TEMPO_ROTATE_WALLET="${TEMPO_ROTATE_WALLET:-0}"
if [ "$TEMPO_ROTATE_WALLET" = "1" ]; then
  if [ -z "${TEMPO_HOME:-}" ]; then
    echo "ERROR: TEMPO_ROTATE_WALLET=1 requires an explicit isolated TEMPO_HOME"
    exit 1
  fi
  if [ -e "$TEMPO_HOME/wallet/store.json" ]; then
    echo "ERROR: refusing to rotate into a non-empty Tempo Accounts store at $TEMPO_HOME/wallet/store.json"
    exit 1
  fi

  echo "Creating a fresh Tempo account and pending access key in store.json"
  ROOT_WALLET_JSON=$("$CAST" wallet new --json)
  ACCESS_WALLET_JSON=$("$CAST" wallet new --json)
  ROOT_PRIVATE_KEY=$(printf '%s' "$ROOT_WALLET_JSON" | jq -r '(.data // .)[0].private_key')
  ACCESS_PRIVATE_KEY=$(printf '%s' "$ACCESS_WALLET_JSON" | jq -r '(.data // .)[0].private_key')
  WALLET=$(printf '%s' "$ROOT_WALLET_JSON" | jq -r '(.data // .)[0].address')
  ACCESS_ADDRESS=$(printf '%s' "$ACCESS_WALLET_JSON" | jq -r '(.data // .)[0].address')
  if [ -z "$ROOT_PRIVATE_KEY" ] || [ "$ROOT_PRIVATE_KEY" = "null" ] ||
     [ -z "$ACCESS_PRIVATE_KEY" ] || [ "$ACCESS_PRIVATE_KEY" = "null" ] ||
     [ -z "$WALLET" ] || [ "$WALLET" = "null" ] ||
     [ -z "$ACCESS_ADDRESS" ] || [ "$ACCESS_ADDRESS" = "null" ]; then
    echo "ERROR: failed to parse an ephemeral key from 'cast wallet new --json'"
    exit 1
  fi
  AUTHORIZATION=$("$CAST" key-authorization sign "$ACCESS_ADDRESS" \
    --chain-id "$CHAIN_ID" \
    --private-key "$ROOT_PRIVATE_KEY" \
    --bind-account "$WALLET")
  "$CAST" tempo import-access-key \
    --account "$WALLET" \
    --access-key "$ACCESS_PRIVATE_KEY" \
    --authorization "$AUTHORIZATION"
else
  STORE_SUMMARY=$("$CAST" --json keychain list)
  WALLET=$(printf '%s' "$STORE_SUMMARY" | jq -r --argjson chain_id "$CHAIN_ID" \
    '[.data[] | select(.chain_id == $chain_id and .has_key == true)][0].wallet_address // empty')
  if [ -z "$WALLET" ]; then
    echo "ERROR: no locally signable chain $CHAIN_ID key found in ${TEMPO_HOME:-$HOME/.tempo}/wallet/store.json"
    exit 1
  fi
fi
echo "Wallet: $WALLET"
echo "RPC:    $RPC_MPP"
echo ""
WALLET_LOWER=$(printf '%s' "$WALLET" | tr '[:upper:]' '[:lower:]')

charge_logs() {
  local from_block="$1"
  "$CAST" logs --json \
    --from-block "$from_block" \
    --to-block latest \
    --address "$TOKEN" \
    'Transfer(address,address,uint256)' \
    "$WALLET" \
    --rpc-url "$RPC"
}

wait_for_new_charge() {
  local from_block="$1"
  local baseline="$2"
  local logs count
  for _ in $(seq 1 30); do
    logs=$(charge_logs "$from_block")
    count=$(printf '%s' "$logs" | jq 'length')
    if [ "$count" -gt "$baseline" ]; then
      printf '%s' "$logs"
      return 0
    fi
    sleep 1
  done
  echo "ERROR: MPP request succeeded but no settled token transfer from $WALLET was found" >&2
  return 1
}

print_latest_charge() {
  local logs="$1"
  local tx amount_hex amount
  tx=$(printf '%s' "$logs" | jq -r '.[-1].transactionHash')
  amount_hex=$(printf '%s' "$logs" | jq -r '.[-1].data')
  amount=$("$CAST" to-dec "$amount_hex")
  echo "Settled Charge: tx=$tx amount=$amount"
}

# 1. Check balance before
echo "=== 1. Balance BEFORE ==="
BEFORE=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC")
echo "$BEFORE"
BEFORE_RAW=$(echo "$BEFORE" | awk '{print $1}')

if [ "$BEFORE_RAW" -lt "$MIN_BALANCE" ] && [ "$TEMPO_AUTO_FUND" = "1" ]; then
  echo "Balance below threshold, requesting faucet funds for $WALLET_LOWER"
  ATTEMPT=0
  while [ "$BEFORE_RAW" -lt "$MIN_BALANCE" ] && [ "$ATTEMPT" -lt "$TEMPO_AUTO_FUND_ATTEMPTS" ]; do
    ATTEMPT=$((ATTEMPT + 1))
    echo "Faucet attempt $ATTEMPT/$TEMPO_AUTO_FUND_ATTEMPTS"
    # Retry on a transient faucet error instead of aborting (set -e).
    if ! "$CAST" rpc tempo_fundAddress "$WALLET_LOWER" --rpc-url "$RPC" >/dev/null; then
      echo "Faucet RPC failed on attempt $ATTEMPT, retrying..."
      sleep 2
      continue
    fi
    sleep 2
    BEFORE=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC")
    echo "$BEFORE"
    BEFORE_RAW=$(echo "$BEFORE" | awk '{print $1}')
  done
fi

if [ "$BEFORE_RAW" -lt "$MIN_BALANCE" ]; then
  echo "ERROR: Wallet balance too low for MPP e2e. Need at least $MIN_BALANCE units of $TOKEN, got $BEFORE_RAW. Refill the CI wallet."
  exit 1
fi

# 2. Pay one Charge through the MPP-gated endpoint.
echo ""
echo "=== 2. cast block-number (via MPP) ==="
FROM_BLOCK=$("$CAST" block-number --rpc-url "$RPC")
BASELINE=$(charge_logs "$FROM_BLOCK" | jq 'length')
BLOCK=$("$CAST" block-number --rpc-url "$RPC_MPP")
echo "Block: $BLOCK"
CHARGE_LOGS=$(wait_for_new_charge "$FROM_BLOCK" "$BASELINE")
print_latest_charge "$CHARGE_LOGS"

# 3. Check balance after
echo ""
echo "=== 3. Balance AFTER ==="
AFTER=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC")
echo "$AFTER"

AFTER_RAW=$(echo "$AFTER" | awk '{print $1}')
SPENT=$((BEFORE_RAW - AFTER_RAW))
echo "Net balance delta: $SPENT units (zero is valid for a self-payment)"

# 4. Verify that a second request is another independent Charge.
echo ""
echo "=== 4. Second cast Charge ==="
BEFORE2=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC" | awk '{print $1}')
FROM_BLOCK2=$("$CAST" block-number --rpc-url "$RPC")
BASELINE2=$(charge_logs "$FROM_BLOCK2" | jq 'length')
BLOCK2=$("$CAST" block-number --rpc-url "$RPC_MPP")
CHARGE_LOGS2=$(wait_for_new_charge "$FROM_BLOCK2" "$BASELINE2")
AFTER2=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC" | awk '{print $1}')
SPENT2=$((BEFORE2 - AFTER2))
echo "Block: $BLOCK2"
print_latest_charge "$CHARGE_LOGS2"
echo "Net balance delta: $SPENT2 units (zero is valid for a self-payment)"

# 5. forge script via MPP
echo ""
echo "=== 5. forge script (via MPP) ==="
MPP_TEST_DIR=$(mktemp -d)
ANVIL_PID=""
cleanup() {
  if [ -n "$ANVIL_PID" ] && kill -0 "$ANVIL_PID" 2>/dev/null; then
    kill "$ANVIL_PID" 2>/dev/null || true
    wait "$ANVIL_PID" 2>/dev/null || true
  fi
  if [ -n "${MPP_TEST_DIR:-}" ] && [ -d "$MPP_TEST_DIR" ]; then
    rm -rf -- "$MPP_TEST_DIR"
  fi
}
trap cleanup EXIT
(cd "$MPP_TEST_DIR" && "$FORGE" init --no-git --quiet)
cat > "$MPP_TEST_DIR/script/Mpp.s.sol" <<'SOL'
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import "forge-std/Script.sol";
contract MppCheck is Script {
    function run() public view {
        console.log("block", block.number);
        console.log("chain", block.chainid);
    }
}
SOL
"$FORGE" script "$MPP_TEST_DIR/script/Mpp.s.sol" --rpc-url "$RPC_MPP" --root "$MPP_TEST_DIR"

# 6. forge test with vm.createSelectFork via MPP
echo ""
echo "=== 6. forge test with createSelectFork (via MPP) ==="
cat > "$MPP_TEST_DIR/test/Mpp.t.sol" <<SOL
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import "forge-std/Test.sol";
contract MppForkTest is Test {
    function test_fork_via_mpp() public {
        vm.createSelectFork("$RPC_MPP");
        assertGt(block.number, 0);
        assertEq(block.chainid, $CHAIN_ID);
    }
}
SOL
"$FORGE" test --match-test test_fork_via_mpp --root "$MPP_TEST_DIR" -vvv

# 7. anvil fork via MPP
echo ""
echo "=== 7. anvil --fork-url (via MPP) ==="
ANVIL_LOG="$MPP_TEST_DIR/anvil.log"
"$ANVIL" --fork-url "$RPC_MPP" --port 8555 >"$ANVIL_LOG" 2>&1 &
ANVIL_PID=$!
ANVIL_READY=0
# A Charge is settled for every fork bootstrap request. On mainnet this cold
# start currently takes roughly 2.5 minutes, so allow five minutes.
for _ in $(seq 1 300); do
  if "$CAST" block-number --rpc-url http://localhost:8555 >/dev/null 2>&1; then
    ANVIL_READY=1
    break
  fi
  if ! kill -0 "$ANVIL_PID" 2>/dev/null; then
    break
  fi
  sleep 1
done
if [ "$ANVIL_READY" != "1" ]; then
  echo "ERROR: anvil did not become ready" >&2
  cat "$ANVIL_LOG" >&2
  exit 1
fi
echo "chain-id: $("$CAST" chain-id --rpc-url http://localhost:8555)"
kill "$ANVIL_PID" 2>/dev/null || true
wait "$ANVIL_PID" 2>/dev/null || true
ANVIL_PID=""

# 8. chisel fork via MPP
echo ""
echo "=== 8. chisel --fork-url (via MPP) ==="
echo 'block.number' | "$CHISEL" --fork-url "$RPC_MPP" 2>&1 | grep -E "Decimal|Type"

echo ""
echo "=== Done ==="
