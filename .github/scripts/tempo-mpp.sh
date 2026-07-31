#!/usr/bin/env bash
# MPP (Machine Payments Protocol) end-to-end test script
#
# Prerequisites:
#   - A Tempo Accounts `store.json`, or `TEMPO_ROTATE_WALLET=1` with an isolated `TEMPO_HOME`
#   - Wallet funded with the configured balance-check token; Accounts may autoswap it into the
#     server-selected channel token
#   - Foundry binaries built: `cargo build --bin cast --bin forge --bin anvil --bin chisel`
#
# Usage:
#   ./.github/scripts/tempo-mpp.sh [binary-dir]
#
# Examples:
#   ./.github/scripts/tempo-mpp.sh                # uses Foundry tools from PATH
#   ./.github/scripts/tempo-mpp.sh ./target/debug # use debug builds

set -euo pipefail

# Moderato requires gateway authentication before returning its paid challenge. Keep the opt-in
# explicit; the channel assertions below still verify that payment was not bypassed.
if [ -n "${MPP_API_KEY:-}" ] && [ "${MPP_ALLOW_API_KEY:-0}" != "1" ]; then
  echo "ERROR: MPP_API_KEY requires MPP_ALLOW_API_KEY=1 for the paid MPP e2e" >&2
  exit 1
fi
if [ "${MPP_ALLOW_API_KEY:-0}" = "1" ] && [ -z "${MPP_API_KEY:-}" ]; then
  echo "ERROR: MPP_ALLOW_API_KEY=1 requires MPP_API_KEY for gateway authentication" >&2
  exit 1
fi

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
FUNDING_TOKEN="${MPP_FUNDING_TOKEN:-${MPP_TOKEN:-0x20c0000000000000000000000000000000000000}}"  # PathUSD on Moderato

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
if ! command -v python3 &>/dev/null; then
  echo "ERROR: python3 is required to inspect the local MPP channel store"
  exit 1
fi

file_sha256() {
  if command -v sha256sum &>/dev/null; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

CHAIN_ID=$("$CAST" chain-id --rpc-url "$RPC")
WALLET_DIR="${TEMPO_HOME:-$HOME/.tempo}/wallet"
STORE_PATH="$WALLET_DIR/store.json"
CHANNELS_DB="$WALLET_DIR/channels.db"
STORE_HASH_BEFORE=""

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
    --private-key "$ROOT_PRIVATE_KEY")
  "$CAST" tempo import-access-key \
    --account "$WALLET" \
    --access-key "$ACCESS_PRIVATE_KEY" \
    --authorization "$AUTHORIZATION"
else
  if [ ! -f "$STORE_PATH" ]; then
    echo "ERROR: Tempo Accounts store not found at $STORE_PATH"
    exit 1
  fi
  STORE_SUMMARY=$("$CAST" --json keychain list)
  ACTIVE_ACCOUNT=$(jq -er '."tempo-cli.store".state.activeAccount' "$STORE_PATH")
  WALLET=$(jq -er --argjson active "$ACTIVE_ACCOUNT" \
    '."tempo-cli.store".state.accounts[$active].address' "$STORE_PATH")
  ACTIVE_KEY_AVAILABLE=$(printf '%s' "$STORE_SUMMARY" | jq -r \
    --argjson chain_id "$CHAIN_ID" \
    --arg wallet "$(printf '%s' "$WALLET" | tr '[:upper:]' '[:lower:]')" \
    'any(.data[]; .chain_id == $chain_id and .has_key == true and (.wallet_address | ascii_downcase) == $wallet)')
  if [ "$ACTIVE_KEY_AVAILABLE" != "true" ]; then
    echo "ERROR: active account $WALLET has no locally signable chain $CHAIN_ID key in $STORE_PATH"
    exit 1
  fi
fi
STORE_SUMMARY=$("$CAST" --json keychain list)
if [ -f "$STORE_PATH" ]; then
  STORE_HASH_BEFORE=$(file_sha256 "$STORE_PATH")
fi
echo "Wallet: $WALLET"
echo "RPC:    $RPC_MPP"
echo ""
WALLET_LOWER=$(printf '%s' "$WALLET" | tr '[:upper:]' '[:lower:]')
RPC_MPP_NORMALIZED="${RPC_MPP%/}"

read_channel_state() {
  if [ ! -s "$CHANNELS_DB" ]; then
    echo "ERROR: MPP request succeeded but $CHANNELS_DB was not created" >&2
    return 1
  fi
  python3 - "$CHANNELS_DB" <<'PY'
import json
import sqlite3
import sys

path = sys.argv[1]
connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
row = connection.execute(
    """
    SELECT channel_id, request_url, payer, token, authorized_signer,
           escrow_contract, chain_id, deposit, cumulative_amount, state
    FROM channels
    WHERE state = 'active' AND session_protocol = 'v2'
    ORDER BY last_used_at DESC
    LIMIT 1
    """
).fetchone()
if row is None:
    raise SystemExit("channels.db has no active TIP-1034 session")
keys = (
    "channel_id",
    "request_url",
    "payer",
    "token",
    "authorized_signer",
    "escrow_contract",
    "chain_id",
    "deposit",
    "cumulative_amount",
    "state",
)
print(json.dumps(dict(zip(keys, row)), separators=(",", ":")))
PY
}

assert_channel_state() {
  local state="$1"
  local payer token signer escrow chain_id request_url channel_id deposit cumulative
  local signer_is_local onchain_state onchain_deposit
  payer=$(printf '%s' "$state" | jq -r '.payer | ascii_downcase')
  token=$(printf '%s' "$state" | jq -r '.token | ascii_downcase')
  signer=$(printf '%s' "$state" | jq -r '.authorized_signer | ascii_downcase')
  escrow=$(printf '%s' "$state" | jq -r '.escrow_contract | ascii_downcase')
  chain_id=$(printf '%s' "$state" | jq -r '.chain_id')
  request_url=$(printf '%s' "$state" | jq -r '.request_url | rtrimstr("/")')
  channel_id=$(printf '%s' "$state" | jq -r '.channel_id')
  deposit=$(printf '%s' "$state" | jq -r '.deposit')
  cumulative=$(printf '%s' "$state" | jq -r '.cumulative_amount')
  if [ "$payer" != "$WALLET_LOWER" ]; then
    echo "ERROR: channels.db payer $payer does not match Accounts wallet $WALLET_LOWER" >&2
    return 1
  fi
  if [[ ! "$token" =~ ^0x20c0[0-9a-f]{36}$ ]]; then
    echo "ERROR: channels.db token $token is not a Tempo TIP-20 address" >&2
    return 1
  fi
  if [ "$escrow" != "0x4d50500000000000000000000000000000000000" ]; then
    echo "ERROR: channels.db escrow $escrow is not the TIP-1034 channel reserve" >&2
    return 1
  fi
  if [ "$chain_id" != "$CHAIN_ID" ]; then
    echo "ERROR: channels.db chain $chain_id does not match RPC chain $CHAIN_ID" >&2
    return 1
  fi
  if [ "$request_url" != "$RPC_MPP_NORMALIZED" ]; then
    echo "ERROR: channels.db request URL $request_url does not match $RPC_MPP_NORMALIZED" >&2
    return 1
  fi
  if [ "$cumulative" -le 0 ] || [ "$deposit" -lt "$cumulative" ]; then
    echo "ERROR: invalid channel accounting: cumulative=$cumulative deposit=$deposit" >&2
    return 1
  fi
  signer_is_local=$(printf '%s' "$STORE_SUMMARY" | jq -r --arg signer "$signer" \
    'any(.data[]; ((.key_address // "") | ascii_downcase) == $signer and .has_key == true)')
  if [ "$signer_is_local" != "true" ]; then
    echo "ERROR: channel signer $signer is not locally available in store.json" >&2
    return 1
  fi
  onchain_state=$("$CAST" call --json \
    0x4D50500000000000000000000000000000000000 \
    'getChannelState(bytes32)((uint96,uint96,uint32))' \
    "$channel_id" \
    --rpc-url "$RPC")
  onchain_deposit=$(printf '%s' "$onchain_state" | jq -r \
    '(if type == "object" then .data else . end)[0][1]')
  if [ "$onchain_deposit" -lt "$deposit" ]; then
    echo "ERROR: channels.db deposit $deposit exceeds on-chain deposit $onchain_deposit" >&2
    return 1
  fi
}

# 1. Check balance before
echo "=== 1. Balance BEFORE ==="
BEFORE=$("$CAST" erc20 balance "$FUNDING_TOKEN" "$WALLET" --rpc-url "$RPC")
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
    BEFORE=$("$CAST" erc20 balance "$FUNDING_TOKEN" "$WALLET" --rpc-url "$RPC")
    echo "$BEFORE"
    BEFORE_RAW=$(echo "$BEFORE" | awk '{print $1}')
  done
fi

if [ "$BEFORE_RAW" -lt "$MIN_BALANCE" ]; then
  echo "ERROR: Wallet balance too low for MPP e2e. Need at least $MIN_BALANCE units of $FUNDING_TOKEN, got $BEFORE_RAW. Refill the CI wallet."
  exit 1
fi

# 2. Pay through a reusable MPP session.
echo ""
echo "=== 2. cast block-number (via MPP) ==="
BLOCK=$("$CAST" block-number --rpc-url "$RPC_MPP")
echo "Block: $BLOCK"
CHANNEL1=$(read_channel_state)
assert_channel_state "$CHANNEL1"
CHANNEL_ID1=$(printf '%s' "$CHANNEL1" | jq -r '.channel_id')
CUMULATIVE1=$(printf '%s' "$CHANNEL1" | jq -r '.cumulative_amount')
DEPOSIT1=$(printf '%s' "$CHANNEL1" | jq -r '.deposit')
echo "Channel: $CHANNEL_ID1 cumulative=$CUMULATIVE1 deposit=$DEPOSIT1"
if [ -e "$WALLET_DIR/sessions.toml" ]; then
  echo "ERROR: legacy sessions.toml was created" >&2
  exit 1
fi

# 3. Check balance after
echo ""
echo "=== 3. Balance AFTER ==="
AFTER=$("$CAST" erc20 balance "$FUNDING_TOKEN" "$WALLET" --rpc-url "$RPC")
echo "$AFTER"

AFTER_RAW=$(echo "$AFTER" | awk '{print $1}')
SPENT=$((BEFORE_RAW - AFTER_RAW))
echo "Net balance delta: $SPENT units (zero is valid for a self-payment)"

# 4. Verify that a second request advances the same reusable channel.
echo ""
echo "=== 4. Second cast session payment ==="
BEFORE2=$("$CAST" erc20 balance "$FUNDING_TOKEN" "$WALLET" --rpc-url "$RPC" | awk '{print $1}')
BLOCK2=$("$CAST" block-number --rpc-url "$RPC_MPP")
AFTER2=$("$CAST" erc20 balance "$FUNDING_TOKEN" "$WALLET" --rpc-url "$RPC" | awk '{print $1}')
SPENT2=$((BEFORE2 - AFTER2))
CHANNEL2=$(read_channel_state)
assert_channel_state "$CHANNEL2"
CHANNEL_ID2=$(printf '%s' "$CHANNEL2" | jq -r '.channel_id')
CUMULATIVE2=$(printf '%s' "$CHANNEL2" | jq -r '.cumulative_amount')
DEPOSIT2=$(printf '%s' "$CHANNEL2" | jq -r '.deposit')
if [ "$CHANNEL_ID2" != "$CHANNEL_ID1" ]; then
  echo "ERROR: second request replaced reusable channel $CHANNEL_ID1 with $CHANNEL_ID2" >&2
  exit 1
fi
if [ "$CUMULATIVE2" -le "$CUMULATIVE1" ]; then
  echo "ERROR: second request did not advance the cumulative voucher" >&2
  exit 1
fi
echo "Block: $BLOCK2"
echo "Channel: $CHANNEL_ID2 cumulative=$CUMULATIVE2 deposit=$DEPOSIT2"
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
# Fork bootstrap emits many paid requests. Allow five minutes for a cold runner.
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

CHANNEL_FINAL=$(read_channel_state)
assert_channel_state "$CHANNEL_FINAL"
CHANNEL_ID_FINAL=$(printf '%s' "$CHANNEL_FINAL" | jq -r '.channel_id')
CUMULATIVE_FINAL=$(printf '%s' "$CHANNEL_FINAL" | jq -r '.cumulative_amount')
DEPOSIT_FINAL=$(printf '%s' "$CHANNEL_FINAL" | jq -r '.deposit')
if [ "$CHANNEL_ID_FINAL" != "$CHANNEL_ID1" ]; then
  echo "ERROR: Foundry tools did not reuse channel $CHANNEL_ID1" >&2
  exit 1
fi
if [ "$CUMULATIVE_FINAL" -le "$CUMULATIVE2" ]; then
  echo "ERROR: Forge/Anvil/Chisel did not advance the cumulative voucher" >&2
  exit 1
fi
if [ -e "$WALLET_DIR/sessions.toml" ]; then
  echo "ERROR: legacy sessions.toml was created" >&2
  exit 1
fi
if [ -n "$STORE_HASH_BEFORE" ]; then
  STORE_HASH_AFTER=$(file_sha256 "$STORE_PATH")
  if [ "$STORE_HASH_AFTER" != "$STORE_HASH_BEFORE" ]; then
    echo "ERROR: MPP requests mutated the Tempo Accounts store" >&2
    exit 1
  fi
fi
echo "Final channel: $CHANNEL_ID_FINAL cumulative=$CUMULATIVE_FINAL deposit=$DEPOSIT_FINAL"

echo ""
echo "=== Done ==="
