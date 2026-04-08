#!/bin/bash
set -euo pipefail

# Deployment checks: forge script and forge create with optional --verify flag
# If VERIFIER_URL is set, adds --verify flag to deployment commands

# Fee token address, defaults to native fee token
FEE_TOKEN="${TEMPO_FEE_TOKEN:-0x20c0000000000000000000000000000000000000}"

# Build fee token args if not using native token (array for safe expansion)
FEE_TOKEN_ARG=()
if [[ "$FEE_TOKEN" != "0x20c0000000000000000000000000000000000000" ]]; then
  FEE_TOKEN_ARG=(--tempo.fee-token "$FEE_TOKEN")
fi

# If VERIFIER_URL is set, add the --verify flag to forge commands
VERIFY_ARG=()
if [[ -n "${VERIFIER_URL:-}" ]]; then
  VERIFY_ARG=(--verify --retries 10 --delay 10)
  echo -e "\n=== USING VERIFIER: $VERIFIER_URL ==="
else
  echo -e "\n=== VERIFIER_URL not set, skipping verification ==="
fi

echo -e "\n=== USING FEE TOKEN: $FEE_TOKEN ==="

echo -e "\n=== INIT TEMPO PROJECT ==="
tmp_dir=$(mktemp -d)
cd "$tmp_dir"
forge init -n tempo tempo-deploy
cd tempo-deploy

if [[ -n "${PRIVATE_KEY:-}" ]]; then
  echo -e "\n=== USING PROVIDED PRIVATE KEY ==="
  PK="$PRIVATE_KEY"
  ADDR="$(cast wallet address "$PK")"
  printf "\naddress: %s\n" "$ADDR"
else
  echo -e "\n=== CREATE AND FUND ADDRESS ==="
  wallet_json="$(cast wallet new --json)"
  ADDR="$(jq -r '.[0].address' <<<"$wallet_json")"
  PK="$(jq -r '.[0].private_key' <<<"$wallet_json")"

  for i in {1..100}; do
    OUT=$(cast rpc tempo_fundAddress "$ADDR" --rpc-url "$ETH_RPC_URL" 2>&1 || true)

    if echo "$OUT" | jq -e 'arrays' >/dev/null 2>&1; then
      echo "$OUT" | jq
      break
    fi

    echo "[$i] $OUT"
    sleep 0.2
  done

  printf "\naddress: %s\nprivate_key: %s\n" "$ADDR" "$PK"

  echo -e "\n=== WAIT FOR BLOCKS TO MINE ==="
  sleep 5
fi

# TODO(upstream): re-enable once forge script fee token validation is fixed
# Currently fails with "invalid fee token: 0x0000000000000000000000000000000000000000"
# echo -e "\n=== FORGE SCRIPT DEPLOY ==="
# forge script ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} script/Mail.s.sol --sig "run(string)" "$(date +%s%N)" --private-key "$PK" --rpc-url "$ETH_RPC_URL" --broadcast ${VERIFY_ARG[@]+"${VERIFY_ARG[@]}"}

# echo -e "\n=== FORGE SCRIPT DEPLOY WITH FEE TOKEN ==="
# forge script ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} script/Mail.s.sol --sig "run(string)" "$(date +%s%N)" --private-key "$PK" --rpc-url "$ETH_RPC_URL" --broadcast ${VERIFY_ARG[@]+"${VERIFY_ARG[@]}"}

echo -e "\n=== FORGE CREATE DEPLOY ==="
forge create ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} src/Mail.sol:Mail --private-key "$PK" --rpc-url "$ETH_RPC_URL" --broadcast ${VERIFY_ARG[@]+"${VERIFY_ARG[@]}"} --constructor-args "$FEE_TOKEN"

echo -e "\n=== FORGE CREATE DEPLOY WITH FEE TOKEN ==="
CHAIN_ID=$(cast chain-id --rpc-url "$ETH_RPC_URL")
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  if [[ "$CHAIN_ID" == "4217" ]]; then
    echo "Skipping alternate fee token test on mainnet (chain 4217)"
  else
    # Test alternate fee tokens only on testnet
    forge create --tempo.fee-token 0x20C0000000000000000000000000000000000002 src/Mail.sol:Mail --private-key "$PK" --rpc-url "$ETH_RPC_URL" --broadcast ${VERIFY_ARG[@]+"${VERIFY_ARG[@]}"} --constructor-args "$FEE_TOKEN"
    forge create --tempo.fee-token 0x20C0000000000000000000000000000000000003 src/Mail.sol:Mail --private-key "$PK" --rpc-url "$ETH_RPC_URL" --broadcast ${VERIFY_ARG[@]+"${VERIFY_ARG[@]}"} --constructor-args "$FEE_TOKEN"
  fi
else
  forge create ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} src/Mail.sol:Mail --private-key "$PK" --rpc-url "$ETH_RPC_URL" --broadcast ${VERIFY_ARG[@]+"${VERIFY_ARG[@]}"} --constructor-args "$FEE_TOKEN"
fi

echo -e "\n=== DEPLOYMENT TESTS COMPLETE ==="
