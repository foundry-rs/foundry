#!/usr/bin/env bash
# MPP (Machine Payments Protocol) end-to-end test script
#
# Prerequisites:
#   - Either `TEMPO_PRIVATE_KEY` is set, or Tempo wallet is configured: `tempo wallet login`
#   - Wallet funded with TEMPO on moderato testnet
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
export MPP_DEPOSIT="${MPP_DEPOSIT:-100000}"
TEMPO_AUTO_FUND="${TEMPO_AUTO_FUND:-0}"
TEMPO_AUTO_FUND_ATTEMPTS="${TEMPO_AUTO_FUND_ATTEMPTS:-3}"
MIN_BALANCE="${MPP_MIN_BALANCE:-$((MPP_DEPOSIT + 50000))}"
RPC_MPP="https://rpc.mpp.moderato.tempo.xyz"
RPC="https://rpc.moderato.tempo.xyz"
TOKEN="0x20c0000000000000000000000000000000000000"  # TEMPO TIP-20

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

KEYS_FILE="${TEMPO_HOME:-$HOME/.tempo}/wallet/keys.toml"
if [ -n "${TEMPO_PRIVATE_KEY:-}" ]; then
  WALLET=$("$CAST" wallet address --private-key "$TEMPO_PRIVATE_KEY")
elif [ -f "$KEYS_FILE" ]; then
  WALLET=$(grep -m1 'wallet_address' "$KEYS_FILE" | sed 's/.*= *"\(.*\)"/\1/')
else
  echo "ERROR: Set TEMPO_PRIVATE_KEY or configure Tempo wallet with: tempo wallet login"
  exit 1
fi
echo "Wallet: $WALLET"
echo "RPC:    $RPC_MPP"
echo ""
WALLET_LOWER=$(printf '%s' "$WALLET" | tr '[:upper:]' '[:lower:]')

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
    "$CAST" rpc tempo_fundAddress "$WALLET_LOWER" --rpc-url "$RPC" >/dev/null
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

# 2. Call block-number through MPP-gated endpoint
echo ""
echo "=== 2. cast block-number (via MPP) ==="
FROM_BLOCK=$("$CAST" block-number --rpc-url "$RPC")
BLOCK=$("$CAST" block-number --rpc-url "$RPC_MPP")
echo "Block: $BLOCK"

# Wait for channel open tx to settle (2 blocks ≈ 6s)
echo "Waiting for channel open tx to settle..."
sleep 6

# 3. Check balance after
echo ""
echo "=== 3. Balance AFTER ==="
AFTER=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC")
echo "$AFTER"

AFTER_RAW=$(echo "$AFTER" | awk '{print $1}')
SPENT=$((BEFORE_RAW - AFTER_RAW))
echo "Spent: $SPENT units (channel deposit + gas)"

# 4. Find and inspect the escrow transaction
echo ""
echo "=== 4. Escrow transaction ==="
TX=$("$CAST" logs --from-block "$FROM_BLOCK" --to-block latest \
  --address 0xe1c4d3dce17bc111181ddf716f75bae49e61a336 \
  --rpc-url "$RPC" | grep transactionHash | tail -1 | awk '{print $2}' || true)

if [ -n "$TX" ]; then
  echo "Tx: $TX"
  "$CAST" tx "$TX" --rpc-url "$RPC"
else
  echo "No new escrow tx (channel reused from previous session)"
fi

# 5. Verify a second call reuses the channel (no new deposit)
echo ""
echo "=== 5. Second call (channel reuse) ==="
BEFORE2=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC" | awk '{print $1}')
BLOCK2=$("$CAST" block-number --rpc-url "$RPC_MPP")
AFTER2=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC" | awk '{print $1}')
SPENT2=$((BEFORE2 - AFTER2))
echo "Block: $BLOCK2"
echo "Spent: $SPENT2 units (should be 0 — channel reused from ~/.tempo/channels.db)"

# 6. forge script via MPP
echo ""
echo "=== 6. forge script (via MPP) ==="
TMPDIR=$(mktemp -d)
trap 'rm -rf $TMPDIR' EXIT
(cd "$TMPDIR" && "$FORGE" init --no-git --quiet)
cat > "$TMPDIR/script/Mpp.s.sol" <<'SOL'
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
VCNT_BEFORE=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
"$FORGE" script "$TMPDIR/script/Mpp.s.sol" --rpc-url "$RPC_MPP" --root "$TMPDIR"
VCNT_AFTER=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
echo "Vouchers paid: +$((VCNT_AFTER - VCNT_BEFORE)) ($((( VCNT_AFTER - VCNT_BEFORE ) / 1000)) RPC calls via MPP)"

# 7. forge test with vm.createSelectFork via MPP
echo ""
echo "=== 7. forge test with createSelectFork (via MPP) ==="
cat > "$TMPDIR/test/Mpp.t.sol" <<SOL
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import "forge-std/Test.sol";
contract MppForkTest is Test {
    function test_fork_via_mpp() public {
        vm.createSelectFork("$RPC_MPP");
        assertGt(block.number, 0);
        assertEq(block.chainid, 42431);
    }
}
SOL
VCNT_BEFORE=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
"$FORGE" test --match-test test_fork_via_mpp --root "$TMPDIR" -vvv
VCNT_AFTER=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
echo "Vouchers paid: +$((VCNT_AFTER - VCNT_BEFORE)) ($((( VCNT_AFTER - VCNT_BEFORE ) / 1000)) RPC calls via MPP)"

# 8. anvil fork via MPP
echo ""
echo "=== 8. anvil --fork-url (via MPP) ==="
VCNT_BEFORE=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
"$ANVIL" --fork-url "$RPC_MPP" --port 8555 --silent &
ANVIL_PID=$!
for _ in $(seq 1 30); do
  if "$CAST" block-number --rpc-url http://localhost:8555 2>/dev/null; then break; fi
  sleep 1
done
echo "chain-id: $("$CAST" chain-id --rpc-url http://localhost:8555)"
kill $ANVIL_PID 2>/dev/null
wait $ANVIL_PID 2>/dev/null
VCNT_AFTER=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
echo "Vouchers paid: +$((VCNT_AFTER - VCNT_BEFORE)) ($((( VCNT_AFTER - VCNT_BEFORE ) / 1000)) RPC calls via MPP)"

# 9. chisel fork via MPP
echo ""
echo "=== 9. chisel --fork-url (via MPP) ==="
VCNT_BEFORE=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
echo 'block.number' | "$CHISEL" --fork-url "$RPC_MPP" 2>&1 | grep -E "Decimal|Type"
VCNT_AFTER=$(sqlite3 ~/.tempo/channels.db "SELECT cumulative_amount FROM channels LIMIT 1")
echo "Vouchers paid: +$((VCNT_AFTER - VCNT_BEFORE)) ($((( VCNT_AFTER - VCNT_BEFORE ) / 1000)) RPC calls via MPP)"

echo ""
echo "=== Done ==="
