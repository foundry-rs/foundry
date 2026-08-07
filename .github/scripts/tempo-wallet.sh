#!/usr/bin/env bash
set -euo pipefail

# Tempo Accounts store tests.
# Exercises --from / --sender resolving a pending access key from store.json
# without requiring a private key on each command.

# Fee token address, defaults to native fee token
FEE_TOKEN="${TEMPO_FEE_TOKEN:-0x20c0000000000000000000000000000000000000}"
TEMPO_WALLET_TEST_HOME=""
PROJECT_DIR=""

cleanup() {
  if [ -n "$PROJECT_DIR" ] && [ -d "$PROJECT_DIR" ]; then
    rm -rf -- "$PROJECT_DIR"
  fi
  if [ -n "$TEMPO_WALLET_TEST_HOME" ] && [ -d "$TEMPO_WALLET_TEST_HOME" ]; then
    rm -rf -- "$TEMPO_WALLET_TEST_HOME"
  fi
}
trap cleanup EXIT

if [ -z "${TEMPO_HOME:-}" ]; then
  TEMPO_WALLET_TEST_HOME="$(mktemp -d)"
  export TEMPO_HOME="$TEMPO_WALLET_TEST_HOME"
fi
if [ -e "$TEMPO_HOME/wallet/store.json" ]; then
  echo "ERROR: refusing to replace the Tempo Accounts store at $TEMPO_HOME/wallet/store.json"
  exit 1
fi
if ! command -v jq &>/dev/null; then
  echo "ERROR: jq is required"
  exit 1
fi

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

wallet_json_field() {
  local wallet_json="$1"
  local field="$2"
  jq -r --arg field "$field" '(.data // .)[0][$field]' <<<"$wallet_json"
}

echo -e "\n=== CREATE TEMPO ACCOUNT AND ACCESS KEY ==="
ROOT_WALLET_JSON="$(cast wallet new --json)"
ACCESS_WALLET_JSON="$(cast wallet new --json)"
WALLET_ADDR="$(wallet_json_field "$ROOT_WALLET_JSON" address)"
ROOT_PRIVATE_KEY="$(wallet_json_field "$ROOT_WALLET_JSON" private_key)"
ACCESS_ADDR="$(wallet_json_field "$ACCESS_WALLET_JSON" address)"
ACCESS_PRIVATE_KEY="$(wallet_json_field "$ACCESS_WALLET_JSON" private_key)"
CHAIN_ID="$(cast chain-id --rpc-url "$ETH_RPC_URL")"
printf "address: %s\n" "$WALLET_ADDR"

echo -e "\n=== IMPORT ACCESS KEY INTO store.json ==="
AUTHORIZATION="$(cast key-authorization sign "$ACCESS_ADDR" \
  --chain-id "$CHAIN_ID" \
  --private-key "$ROOT_PRIVATE_KEY" \
  --bind-account "$WALLET_ADDR")"
cast tempo import-access-key \
  --account "$WALLET_ADDR" \
  --access-key "$ACCESS_PRIVATE_KEY" \
  --authorization "$AUTHORIZATION"
unset ROOT_WALLET_JSON ACCESS_WALLET_JSON ROOT_PRIVATE_KEY ACCESS_PRIVATE_KEY AUTHORIZATION
echo "Written to $TEMPO_HOME/wallet/store.json"

echo "=== Wallet: $WALLET_ADDR ==="
echo "=== RPC:    $ETH_RPC_URL ==="
echo "=== Fee:    $FEE_TOKEN ==="

echo -e "\n=== FUND WALLET ==="
fund_and_wait "$WALLET_ADDR"

echo -e "\n=== CAST SEND WITH --from (Tempo Accounts store) ==="
cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
  --from "$WALLET_ADDR"

echo -e "\n=== CAST ERC20 TRANSFER WITH --from (Tempo Accounts store) ==="
cast erc20 transfer ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} \
  "$FEE_TOKEN" \
  0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 100 \
  --rpc-url "$ETH_RPC_URL" --from "$WALLET_ADDR"

echo -e "\n=== FORGE SCRIPT WITH --sender (Tempo Accounts store) ==="
PROJECT_DIR="$(mktemp -d)"
cd "$PROJECT_DIR"
forge init -n tempo tempo-wallet-test --quiet
cd tempo-wallet-test

cat > script/TempoAccounts.s.sol <<'SOL'
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import "forge-std/Script.sol";
interface TIP20 {
    function transfer(address to, uint256 amount) external returns (bool);
}
contract TempoAccountsScript is Script {
    function run(address token) external {
        vm.startBroadcast();
        require(TIP20(token).transfer(0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F, 1));
        vm.stopBroadcast();
    }
}
SOL
forge script ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} script/TempoAccounts.s.sol \
  --sig "run(address)" "$FEE_TOKEN" \
  --sender "$WALLET_ADDR" --rpc-url "$ETH_RPC_URL" --broadcast

echo -e "\n=== TEMPO WALLET TESTS COMPLETE ==="
