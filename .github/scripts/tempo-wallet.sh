#!/bin/bash
set -euo pipefail

# Tempo wallet keys.toml fallback tests
# Exercises --from / --sender resolving the signer from ~/.tempo/wallet/keys.toml
# without requiring --private-key or --tempo.access-key.
#
# Creates a fresh direct-mode wallet (wallet_address == key_address) and writes
# a keys.toml for it, so the test is self-contained and doesn't require a
# pre-provisioned keychain entry.

# Fee token address, defaults to native fee token
FEE_TOKEN="${TEMPO_FEE_TOKEN:-0x20c0000000000000000000000000000000000000}"

FEE_TOKEN_ARG=()
if [[ "$FEE_TOKEN" != "0x20c0000000000000000000000000000000000000" ]]; then
  FEE_TOKEN_ARG=(--tempo.fee-token "$FEE_TOKEN")
fi

# Fund an address and wait for the fee token balance to be non-zero
fund_and_wait() {
  local addr="$1"
  for i in {1..100}; do
    OUT=$(cast rpc tempo_fundAddress "$addr" --rpc-url "$ETH_RPC_URL" 2>&1 || true)
    if echo "$OUT" | jq -e 'arrays' >/dev/null 2>&1; then
      echo "$OUT" | jq
      break
    fi
    echo "[$i] $OUT"
    sleep 0.2
  done
  echo "Waiting for $addr to be funded..."
  for i in {1..30}; do
    BAL=$(cast call --rpc-url "$ETH_RPC_URL" "$FEE_TOKEN" 'balanceOf(address)(uint256)' "$addr" 2>/dev/null || echo "0")
    if [[ "$BAL" != "0" && -n "$BAL" ]]; then
      echo "Funded with $BAL fee tokens"
      return 0
    fi
    if [[ $i -eq 30 ]]; then
      echo "ERROR: Funding timed out for $addr"
      exit 1
    fi
    sleep 1
  done
}

echo -e "\n=== CREATE DIRECT-MODE WALLET ==="
wallet_json="$(cast wallet new --json)"
WALLET_ADDR="$(jq -r '.[0].address' <<<"$wallet_json")"
WALLET_PK="$(jq -r '.[0].private_key' <<<"$wallet_json")"
printf "address: %s\n" "$WALLET_ADDR"

echo -e "\n=== WRITE keys.toml ==="
mkdir -p "${TEMPO_HOME:-$HOME/.tempo}/wallet"
cat > "${TEMPO_HOME:-$HOME/.tempo}/wallet/keys.toml" <<TOML
[[keys]]
wallet_address = "$WALLET_ADDR"
key = "$WALLET_PK"
TOML
echo "Written to ${TEMPO_HOME:-$HOME/.tempo}/wallet/keys.toml"

echo "=== Wallet: $WALLET_ADDR ==="
echo "=== RPC:    $ETH_RPC_URL ==="
echo "=== Fee:    $FEE_TOKEN ==="

echo -e "\n=== FUND WALLET ==="
fund_and_wait "$WALLET_ADDR"

echo -e "\n=== CAST SEND WITH --from (keys.toml fallback) ==="
cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
  --from "$WALLET_ADDR"

echo -e "\n=== CAST ERC20 TRANSFER WITH --from (keys.toml fallback) ==="
cast erc20 transfer ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} \
  "$FEE_TOKEN" \
  0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 100 \
  --rpc-url "$ETH_RPC_URL" --from "$WALLET_ADDR"

echo -e "\n=== FORGE CREATE WITH --from (keys.toml fallback) ==="
tmp_dir=$(mktemp -d)
cd "$tmp_dir"
forge init -n tempo tempo-wallet-test --quiet
cd tempo-wallet-test

forge create ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} src/Mail.sol:Mail \
  --from "$WALLET_ADDR" --rpc-url "$ETH_RPC_URL" --broadcast \
  --constructor-args "$FEE_TOKEN"

echo -e "\n=== FORGE SCRIPT WITH --sender (keys.toml fallback) ==="
forge script ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} script/Mail.s.sol \
  --sig "run(string)" "$(date +%s%N)" \
  --sender "$WALLET_ADDR" --rpc-url "$ETH_RPC_URL" --broadcast

echo -e "\n=== TEMPO WALLET TESTS COMPLETE ==="
