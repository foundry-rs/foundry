#!/usr/bin/env bash
# MPP (Machine Payments Protocol) end-to-end test script
#
# Prerequisites:
#   - Tempo wallet configured: `tempo wallet login`
#   - Wallet funded with TEMPO on moderato testnet
#   - cast binary built: `cargo build --bin cast`
#
# Usage:
#   ./scripts/mpp-test.sh [cast-binary]
#
# Examples:
#   ./scripts/mpp-test.sh                           # uses ./target/debug/cast
#   ./scripts/mpp-test.sh ./target/release/cast      # use release build

set -euo pipefail

CAST="${1:-cast}"
RPC_MPP="https://rpc.mpp.moderato.tempo.xyz"
RPC="https://rpc.moderato.tempo.xyz"
TOKEN="0x20c0000000000000000000000000000000000000"  # TEMPO TIP-20

if ! command -v "$CAST" &>/dev/null; then
  echo "ERROR: cast binary not found at '$CAST'. Install with: foundryup"
  exit 1
fi

# Discover wallet address from keys.toml
KEYS_FILE="${TEMPO_HOME:-$HOME/.tempo}/wallet/keys.toml"
if [ ! -f "$KEYS_FILE" ]; then
  echo "ERROR: Tempo wallet not configured. Run: tempo wallet login"
  exit 1
fi
WALLET=$(grep -m1 'wallet_address' "$KEYS_FILE" | sed 's/.*= *"\(.*\)"/\1/')
echo "Wallet: $WALLET"
echo "RPC:    $RPC_MPP"
echo ""

# 1. Check balance before
echo "=== 1. Balance BEFORE ==="
BEFORE=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC")
echo "$BEFORE"

# 2. Call block-number through MPP-gated endpoint
echo ""
echo "=== 2. cast block-number (via MPP) ==="
FROM_BLOCK=$("$CAST" block-number --rpc-url "$RPC")
BLOCK=$("$CAST" block-number --rpc-url "$RPC_MPP")
echo "Block: $BLOCK"

# 3. Check balance after
echo ""
echo "=== 3. Balance AFTER ==="
AFTER=$("$CAST" erc20 balance "$TOKEN" "$WALLET" --rpc-url "$RPC")
echo "$AFTER"

BEFORE_RAW=$(echo "$BEFORE" | awk '{print $1}')
AFTER_RAW=$(echo "$AFTER" | awk '{print $1}')
SPENT=$((BEFORE_RAW - AFTER_RAW))
echo "Spent: $SPENT units (channel deposit + gas)"

# 4. Find and inspect the escrow transaction
echo ""
echo "=== 4. Escrow transaction ==="
TX=$("$CAST" logs --from-block "$FROM_BLOCK" --to-block latest \
  --address 0xe1c4d3dce17bc111181ddf716f75bae49e61a336 \
  --rpc-url "$RPC" | grep transactionHash | tail -1 | awk '{print $2}')

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
echo "Spent: $SPENT2 units (should be 0 — channel reused from ~/.tempo/foundry/channels.json)"

echo ""
echo "=== Done ==="
