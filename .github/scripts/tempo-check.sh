#!/bin/bash
set -euo pipefail

# Get the directory where this script lives
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Non-verification tempo checks: local tests, fork tests, cast commands, DEX operations

# Hardfork version, defaults to T3 (latest features)
HARDFORK="${TEMPO_HARDFORK:-T3}"

# Fee token address, defaults to native fee token
FEE_TOKEN="${TEMPO_FEE_TOKEN:-0x20c0000000000000000000000000000000000000}"

# Build fee token args if not using native token (array for safe expansion)
FEE_TOKEN_ARG=()
if [[ "$FEE_TOKEN" != "0x20c0000000000000000000000000000000000000" ]]; then
  FEE_TOKEN_ARG=(--tempo.fee-token "$FEE_TOKEN")
fi

echo -e "\n=== USING HARDFORK: $HARDFORK ==="
echo -e "=== USING FEE TOKEN: $FEE_TOKEN ==="

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

echo -e "\n=== INIT TEMPO PROJECT ==="
tmp_dir=$(mktemp -d)
cd "$tmp_dir"
forge init -n tempo tempo-check
cd tempo-check

# TODO(upstream): re-enable once local (non-RPC) Tempo precompiles are supported
# Currently fails with OpcodeNotFound on fee manager precompile
# echo -e "\n=== FORGE TEST (LOCAL) ==="
# TEMPO_FEE_TOKEN='' forge test

# echo -e "\n=== FORGE SCRIPT (LOCAL) ==="
# TEMPO_FEE_TOKEN='' forge script script/Mail.s.sol --sig "run(string)" "$(date +%s%N)"

echo -e "\n=== START TEMPO DEVNET TESTS ==="

# Export fee token for fork tests (templates use vm.envOr to read it)
export TEMPO_FEE_TOKEN="$FEE_TOKEN"

echo -e "\n=== TEMPO VERSION ==="
cast client --rpc-url "$ETH_RPC_URL"

# TODO(upstream): re-enable once fee token validation is fixed for devnet forge test/script
# Currently fails with "invalid fee token: 0x0000000000000000000000000000000000000000"
# echo -e "\n=== FORGE TEST (DEVNET) ==="
# forge test --rpc-url "$ETH_RPC_URL"

# echo -e "\n=== FORGE SCRIPT (DEVNET) ==="
# forge script ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} script/Mail.s.sol --sig "run(string)" "$(date +%s%N)" --rpc-url "$ETH_RPC_URL"

echo -e "\n=== CREATE AND FUND ADDRESS ==="
wallet_json="$(cast wallet new --json)"
ADDR="$(jq -r '.[0].address' <<<"$wallet_json")"
PK="$(jq -r '.[0].private_key' <<<"$wallet_json")"
printf "address: %s\nprivate_key: %s\n" "$ADDR" "$PK"
fund_and_wait "$ADDR"

echo -e "\n=== ADD AlphaUSD FEE TOKEN LIQUIDITY ==="
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  cast send 0xfeec000000000000000000000000000000000000 'mint(address,address,uint256,address)' 0x20C0000000000000000000000000000000000001 0x20C0000000000000000000000000000000000000 1000000000 0x6c4143BEd3A13cf9E5E43d45C60aD816FC091d0c --private-key "$PK" --rpc-url "$ETH_RPC_URL"
else
  echo "skipped (custom fee token set)"
fi

echo -e "\n=== ADD BetaUSD FEE TOKEN LIQUIDITY ==="
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  cast send 0xfeec000000000000000000000000000000000000 'mint(address,address,uint256,address)' 0x20C0000000000000000000000000000000000002 0x20C0000000000000000000000000000000000000 1000000000 0x6c4143BEd3A13cf9E5E43d45C60aD816FC091d0c --private-key "$PK" --rpc-url "$ETH_RPC_URL"
else
  echo "skipped (custom fee token set)"
fi

echo -e "\n=== ADD ThetaUSD FEE TOKEN LIQUIDITY ==="
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  cast send 0xfeec000000000000000000000000000000000000 'mint(address,address,uint256,address)' 0x20C0000000000000000000000000000000000003 0x20C0000000000000000000000000000000000000 1000000000 0x6c4143BEd3A13cf9E5E43d45C60aD816FC091d0c --private-key "$PK" --rpc-url "$ETH_RPC_URL"
else
  echo "skipped (custom fee token set)"
fi

echo -e "\n=== CAST ERC20 TRANSFER WITH FEE TOKEN ==="
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  cast erc20 transfer --tempo.fee-token 0x20C0000000000000000000000000000000000002 0x20c0000000000000000000000000000000000002 0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 123456 --rpc-url "$ETH_RPC_URL" --private-key "$PK"
  cast erc20 transfer --tempo.fee-token 0x20C0000000000000000000000000000000000003 0x20c0000000000000000000000000000000000002 0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 123456 --rpc-url "$ETH_RPC_URL" --private-key "$PK"
else
  cast erc20 transfer ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} "${FEE_TOKEN}" 0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 123456 --rpc-url "$ETH_RPC_URL" --private-key "$PK"
fi

echo -e "\n=== CAST ERC20 APPROVE WITH FEE TOKEN ==="
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  cast erc20 approve --tempo.fee-token 0x20C0000000000000000000000000000000000002 0x20c0000000000000000000000000000000000002 0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 123456 --rpc-url "$ETH_RPC_URL" --private-key "$PK"
  cast erc20 approve --tempo.fee-token 0x20C0000000000000000000000000000000000003 0x20c0000000000000000000000000000000000002 0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 123456 --rpc-url "$ETH_RPC_URL" --private-key "$PK"
else
  echo "skipped (custom fee token set)"
fi

echo -e "\n=== CAST SEND WITH FEE TOKEN ==="
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  cast send --tempo.fee-token 0x20C0000000000000000000000000000000000002 --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK"
  cast send --tempo.fee-token 0x20C0000000000000000000000000000000000003 --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK"
else
  cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK"
fi

echo -e "\n=== CAST MKTX WITH FEE TOKEN ==="
cast mktx ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK"

echo -e "\n=== CAST MKTX WITH NONCE-KEY (2D Nonce) ==="
# Each nonce-key has its own nonce sequence starting at 0
cast mktx ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" --nonce 0 --tempo.nonce-key 1

echo -e "\n=== CAST SEND WITH NONCE-KEY (2D Nonce) ==="
# Use a different nonce-key (2) with nonce 0 since each key starts fresh
cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" --nonce 0 --tempo.nonce-key 2

echo -e "\n=== CAST MKTX WITH EXPIRING NONCE (TIP-1009) ==="
# Use the node's block timestamp to avoid clock skew between CI runner and devnet.
BLOCK_TS=$(cast block latest --rpc-url "$ETH_RPC_URL" -f timestamp)
cast mktx ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" --tempo.expiring-nonce --tempo.valid-before "$((BLOCK_TS + 25))"

echo -e "\n=== CAST SEND WITH EXPIRING NONCE (TIP-1009) ==="
BLOCK_TS=$(cast block latest --rpc-url "$ETH_RPC_URL" -f timestamp)
cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" --tempo.expiring-nonce --tempo.valid-before "$((BLOCK_TS + 25))"

echo -e "\n=== CAST MKTX WITH EXPIRING NONCE + VALID-AFTER ==="
BLOCK_TS=$(cast block latest --rpc-url "$ETH_RPC_URL" -f timestamp)
cast mktx ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" --tempo.expiring-nonce --tempo.valid-before "$((BLOCK_TS + 25))" --tempo.valid-after "$((BLOCK_TS + 5))"

echo -e "\n=== CAST SEND WITH EXPIRING NONCE + VALID-AFTER ==="
sleep 6  # Wait for valid_after to pass
BLOCK_TS=$(cast block latest --rpc-url "$ETH_RPC_URL" -f timestamp)
cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" --tempo.expiring-nonce --tempo.valid-before "$((BLOCK_TS + 25))" --tempo.valid-after "$((BLOCK_TS - 1))"

# TODO(upstream): re-enable access key + keychain tests once T3 authorizeKey is supported on devnet
# Currently fails with UnknownFunctionSelector on T3 authorizeKey signature
# echo -e "\n=== SETUP ACCESS KEY ==="
# Create an access key for testing
# access_wallet_json="$(cast wallet new --json)"
# ACCESS_KEY="$(jq -r '.[0].private_key' <<<"$access_wallet_json")"
# ACCESS_KEY_ADDR="$(jq -r '.[0].address' <<<"$access_wallet_json")"
# printf "Access key address: %s\n" "$ACCESS_KEY_ADDR"

# Authorize the access key on-chain first (required for gas estimation)
# Account Keychain precompile: 0xAAAAAAAA00000000000000000000000000000000
# SignatureType: 0 = Secp256k1, Expiry: 1893456000 (year 2030), enforceLimits: false, limits: [], allowAnyCalls: true
# if [[ "$HARDFORK" == "T2" ]]; then
#   # Legacy: authorizeKey with flat params (pre-T3)
#   cast send --rpc-url "$ETH_RPC_URL" 0xAAAAAAAA00000000000000000000000000000000 \
#     'authorizeKey(address,uint8,uint64,bool,(address,uint256)[])' \
#     "$ACCESS_KEY_ADDR" 0 1893456000 false "[]" \
#     --private-key "$PK"
# else
#   # TIP-1011 (T3+): authorizeKey takes a KeyRestrictions struct
#   # KeyRestrictions = (uint64 expiry, bool enforceLimits, TokenLimit[] limits, bool allowAnyCalls, CallScope[] allowedCalls)
#   # TokenLimit = (address token, uint256 amount, uint64 period)
#   # CallScope = (address target, SelectorRule[] selectorRules)
#   # SelectorRule = (bytes4 selector, address[] recipients)
#   cast send --rpc-url "$ETH_RPC_URL" 0xAAAAAAAA00000000000000000000000000000000 \
#     'authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[])) ' \
#     "$ACCESS_KEY_ADDR" 0 "(1893456000,false,[],true,[])" \
#     --private-key "$PK"
# fi

# Fund the access key address (needed for gas)
# fund_and_wait "$ACCESS_KEY_ADDR"

# echo -e "\n=== CAST MKTX WITH ACCESS-KEY ==="
# Use original address as root account (access key signs on behalf of root)
# cast mktx ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --tempo.access-key "$ACCESS_KEY" --tempo.root-account "$ADDR"

# echo -e "\n=== CAST SEND WITH ACCESS-KEY ==="
# Send transaction using the access key (Keychain signature wrapped in AA transaction)
# cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --tempo.access-key "$ACCESS_KEY" --tempo.root-account "$ADDR"

# TODO(upstream): re-enable once cast keychain is fixed
# --- cast keychain subcommand tests ---

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE ==="
# kc_wallet_json="$(cast wallet new --json)"
# KC_KEY_PK="$(jq -r '.[0].private_key' <<<"$kc_wallet_json")"
# KC_KEY_ADDR="$(jq -r '.[0].address' <<<"$kc_wallet_json")"
# printf "Keychain key address: %s\n" "$KC_KEY_ADDR"

# cast keychain auth "$KC_KEY_ADDR" secp256k1 1893456000 \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}

# echo -e "\n=== CAST KEYCHAIN: KEY-INFO ==="
# KC_INFO=$(cast keychain info "$ADDR" "$KC_KEY_ADDR" --rpc-url "$ETH_RPC_URL")
# echo "$KC_INFO"
# echo "$KC_INFO" | grep -q "secp256k1"

# echo -e "\n=== CAST KEYCHAIN: KEY-INFO --json ==="
# KC_INFO_JSON=$(cast keychain info "$ADDR" "$KC_KEY_ADDR" --rpc-url "$ETH_RPC_URL" --json)
# echo "$KC_INFO_JSON" | jq -e '.signatureType == "secp256k1"'

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE WITH LIMIT ==="
# kc_limited_json="$(cast wallet new --json)"
# KC_LIMITED_ADDR="$(jq -r '.[0].address' <<<"$kc_limited_json")"
# cast keychain auth "$KC_LIMITED_ADDR" secp256k1 1893456000 \
#   --limit "$FEE_TOKEN:1000000000" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}

# echo -e "\n=== CAST KEYCHAIN: REMAINING-LIMIT ==="
# KC_REMAINING=$(cast keychain rl "$ADDR" "$KC_LIMITED_ADDR" "$FEE_TOKEN" --rpc-url "$ETH_RPC_URL")
# echo "Remaining: $KC_REMAINING"
# [[ "$KC_REMAINING" != "0" ]] || { echo "ERROR: expected non-zero limit"; exit 1; }

# echo -e "\n=== CAST KEYCHAIN: REMAINING-LIMIT --json ==="
# KC_REMAINING_JSON=$(cast keychain rl "$ADDR" "$KC_LIMITED_ADDR" "$FEE_TOKEN" --rpc-url "$ETH_RPC_URL" --json)
# echo "$KC_REMAINING_JSON" | jq -e '. != "0"'

# echo -e "\n=== CAST KEYCHAIN: UPDATE-LIMIT ==="
# cast keychain ul "$KC_LIMITED_ADDR" "$FEE_TOKEN" 500000000 \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}

# echo -e "\n=== CAST KEYCHAIN: VERIFY UPDATE-LIMIT ==="
# KC_UPDATED=$(cast keychain rl "$ADDR" "$KC_LIMITED_ADDR" "$FEE_TOKEN" --rpc-url "$ETH_RPC_URL")
# echo "Remaining after update: $KC_UPDATED"
# [[ "$KC_UPDATED" == "500000000" ]] || { echo "ERROR: expected 500000000 after update-limit, got $KC_UPDATED"; exit 1; }

# echo -e "\n=== CAST KEYCHAIN: REVOKE ==="
# cast keychain rev "$KC_KEY_ADDR" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}

# Verify revocation
# KC_INFO_REV=$(cast keychain info "$ADDR" "$KC_KEY_ADDR" --rpc-url "$ETH_RPC_URL")
# echo "$KC_INFO_REV"
# echo "$KC_INFO_REV" | grep -q "true"

# echo -e "\n=== CAST KEYCHAIN: REVOKED KEY REJECTION ==="
# Fund the revoked key so failure is due to revocation, not gas
# fund_and_wait "$KC_KEY_ADDR"
# if cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
#   --tempo.access-key "$KC_KEY_PK" --tempo.root-account "$ADDR" 2>&1; then
#   echo "ERROR: revoked key should have been rejected"
#   exit 1
# fi
# echo "OK: revoked key correctly rejected"

# echo -e "\n=== CAST KEYCHAIN: DUPLICATE AUTHORIZE REJECTION ==="
# Try to authorize KC_LIMITED_ADDR again — should fail with KeyAlreadyExists
# if cast keychain auth "$KC_LIMITED_ADDR" secp256k1 1893456000 \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} 2>&1; then
#   echo "ERROR: duplicate authorize should have been rejected"
#   exit 1
# fi
# echo "OK: duplicate authorize correctly rejected"

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE WITH --scope (ADDRESS ONLY, UNRESTRICTED) ==="
# kc_scoped_json="$(cast wallet new --json)"
# KC_SCOPED_PK="$(jq -r '.[0].private_key' <<<"$kc_scoped_json")"
# KC_SCOPED_ADDR="$(jq -r '.[0].address' <<<"$kc_scoped_json")"
# cast keychain auth "$KC_SCOPED_ADDR" secp256k1 1893456000 \
#   --scope 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}

# echo -e "\n=== CAST KEYCHAIN: SCOPE ALLOWED TARGET ==="
# fund_and_wait "$KC_SCOPED_ADDR"
# cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
#   --tempo.access-key "$KC_SCOPED_PK" --tempo.root-account "$ADDR"
# echo "OK: scoped key allowed to call permitted target"

# echo -e "\n=== CAST KEYCHAIN: SCOPE BLOCKED TARGET ==="
# if cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 'doesNotExist()' \
#   --tempo.access-key "$KC_SCOPED_PK" --tempo.root-account "$ADDR" 2>&1; then
#   echo "ERROR: scoped key should have been blocked for disallowed target"
#   exit 1
# fi
# echo "OK: scoped key correctly blocked for disallowed target"

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE WITH --scope + SELECTORS ==="
# kc_sel_json="$(cast wallet new --json)"
# KC_SEL_PK="$(jq -r '.[0].private_key' <<<"$kc_sel_json")"
# KC_SEL_ADDR="$(jq -r '.[0].address' <<<"$kc_sel_json")"
# cast keychain auth "$KC_SEL_ADDR" secp256k1 1893456000 \
#   --scope "$FEE_TOKEN:transfer,approve" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}
# echo "OK: authorized key with selector-scoped restrictions"

# echo -e "\n=== CAST KEYCHAIN: SELECTOR-SCOPED TRANSFER ALLOWED ==="
# fund_and_wait "$KC_SEL_ADDR"
# cast erc20 transfer ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} "$FEE_TOKEN" \
#   0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 100 \
#   --rpc-url "$ETH_RPC_URL" \
#   --tempo.access-key "$KC_SEL_PK" --tempo.root-account "$ADDR"
# echo "OK: selector-scoped key allowed transfer on permitted TIP-20"

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE WITH --scopes JSON ==="
# kc_json_json="$(cast wallet new --json)"
# KC_JSON_ADDR="$(jq -r '.[0].address' <<<"$kc_json_json")"
# cast keychain auth "$KC_JSON_ADDR" secp256k1 1893456000 \
#   --scopes "[{\"target\":\"$FEE_TOKEN\",\"selectors\":[\"transfer\"]},{\"target\":\"0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D\"}]" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}
# echo "OK: authorized key with --scopes JSON"

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE WITH MULTIPLE LIMITS ==="
# kc_multi_json="$(cast wallet new --json)"
# KC_MULTI_ADDR="$(jq -r '.[0].address' <<<"$kc_multi_json")"
# cast keychain auth "$KC_MULTI_ADDR" secp256k1 1893456000 \
#   --limit "$FEE_TOKEN:1000000" \
#   --limit "0x20C0000000000000000000000000000000000001:2000000" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}

# Verify both limits
# KC_MULTI_RL1=$(cast keychain rl "$ADDR" "$KC_MULTI_ADDR" "$FEE_TOKEN" --rpc-url "$ETH_RPC_URL")
# KC_MULTI_RL2=$(cast keychain rl "$ADDR" "$KC_MULTI_ADDR" 0x20C0000000000000000000000000000000000001 --rpc-url "$ETH_RPC_URL")
# echo "Limit 1: $KC_MULTI_RL1 (expected 1000000), Limit 2: $KC_MULTI_RL2 (expected 2000000)"
# [[ "$KC_MULTI_RL1" == "1000000" ]] || { echo "ERROR: limit 1 mismatch"; exit 1; }
# [[ "$KC_MULTI_RL2" == "2000000" ]] || { echo "ERROR: limit 2 mismatch"; exit 1; }

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE WITH RAW HEX SELECTOR ==="
# kc_hex_json="$(cast wallet new --json)"
# KC_HEX_PK="$(jq -r '.[0].private_key' <<<"$kc_hex_json")"
# KC_HEX_ADDR="$(jq -r '.[0].address' <<<"$kc_hex_json")"
# increment() selector = 0xd09de08a
# cast keychain auth "$KC_HEX_ADDR" secp256k1 1893456000 \
#   --scope "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:0xd09de08a" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}
# echo "OK: authorized key with raw hex selector"

# echo -e "\n=== CAST KEYCHAIN: RAW HEX SELECTOR ALLOWED ==="
# fund_and_wait "$KC_HEX_ADDR"
# cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
#   --tempo.access-key "$KC_HEX_PK" --tempo.root-account "$ADDR"
# echo "OK: raw hex selector key allowed to call increment()"

# echo -e "\n=== CAST KEYCHAIN: SET-SCOPE ==="
# Create a new unrestricted key, then add scope restrictions via set-scope
# kc_ss_json="$(cast wallet new --json)"
# KC_SS_PK="$(jq -r '.[0].private_key' <<<"$kc_ss_json")"
# KC_SS_ADDR="$(jq -r '.[0].address' <<<"$kc_ss_json")"
# cast keychain auth "$KC_SS_ADDR" secp256k1 1893456000 \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}

# Now restrict it to only the counter contract
# cast keychain ss "$KC_SS_ADDR" \
#   --scope 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}
# echo "OK: set-scope applied"

# echo -e "\n=== CAST KEYCHAIN: SET-SCOPE ALLOWED ==="
# fund_and_wait "$KC_SS_ADDR"
# cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
#   --tempo.access-key "$KC_SS_PK" --tempo.root-account "$ADDR"
# echo "OK: set-scope key allowed to call permitted target"

# echo -e "\n=== CAST KEYCHAIN: SET-SCOPE BLOCKED ==="
# if cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 'doesNotExist()' \
#   --tempo.access-key "$KC_SS_PK" --tempo.root-account "$ADDR" 2>&1; then
#   echo "ERROR: set-scope key should have been blocked for disallowed target"
#   exit 1
# fi
# echo "OK: set-scope key correctly blocked for disallowed target"

# echo -e "\n=== CAST KEYCHAIN: REMOVE-SCOPE (BEFORE — CALL SUCCEEDS) ==="
# cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
#   --tempo.access-key "$KC_SS_PK" --tempo.root-account "$ADDR"
# echo "OK: call to scoped target succeeds before remove-scope"

# echo -e "\n=== CAST KEYCHAIN: REMOVE-SCOPE ==="
# cast keychain rs "$KC_SS_ADDR" 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}
# echo "OK: remove-scope applied"

# echo -e "\n=== CAST KEYCHAIN: REMOVE-SCOPE (AFTER — CALL FAILS) ==="
# if cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' \
#   --tempo.access-key "$KC_SS_PK" --tempo.root-account "$ADDR" 2>&1; then
#   echo "ERROR: call should have been blocked after remove-scope"
#   exit 1
# fi
# echo "OK: call correctly blocked after remove-scope"

# echo -e "\n=== CAST KEYCHAIN: AUTHORIZE WITH RECIPIENT RESTRICTION ==="
# kc_recip_json="$(cast wallet new --json)"
# KC_RECIP_PK="$(jq -r '.[0].private_key' <<<"$kc_recip_json")"
# KC_RECIP_ADDR="$(jq -r '.[0].address' <<<"$kc_recip_json")"
# Only allow transfer to a specific recipient
# ALLOWED_RECIPIENT="0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F"
# cast keychain auth "$KC_RECIP_ADDR" secp256k1 1893456000 \
#   --scope "$FEE_TOKEN:transfer@$ALLOWED_RECIPIENT" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}
# echo "OK: authorized key with recipient-restricted transfer"

# echo -e "\n=== CAST KEYCHAIN: RECIPIENT-SCOPED TRANSFER ALLOWED ==="
# fund_and_wait "$KC_RECIP_ADDR"
# cast erc20 transfer ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} "$FEE_TOKEN" \
#   "$ALLOWED_RECIPIENT" 100 \
#   --rpc-url "$ETH_RPC_URL" \
#   --tempo.access-key "$KC_RECIP_PK" --tempo.root-account "$ADDR"
# echo "OK: recipient-scoped transfer allowed to permitted recipient"

# echo -e "\n=== CAST KEYCHAIN: --scopes JSON WITH RECIPIENTS ==="
# kc_jsonr_json="$(cast wallet new --json)"
# KC_JSONR_ADDR="$(jq -r '.[0].address' <<<"$kc_jsonr_json")"
# cast keychain auth "$KC_JSONR_ADDR" secp256k1 1893456000 \
#   --scopes "[{\"target\":\"$FEE_TOKEN\",\"selectors\":[{\"selector\":\"transfer\",\"recipients\":[\"$ALLOWED_RECIPIENT\"]}]},{\"target\":\"0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D\"}]" \
#   --rpc-url "$ETH_RPC_URL" --private-key "$PK" ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"}
# echo "OK: authorized key with --scopes JSON including recipients"

echo -e "\n=== SETUP SPONSOR ==="
# Create a sponsor wallet for testing sponsored (gasless) transactions
sponsor_wallet_json="$(cast wallet new --json)"
SPONSOR_PK="$(jq -r '.[0].private_key' <<<"$sponsor_wallet_json")"
SPONSOR_ADDR="$(jq -r '.[0].address' <<<"$sponsor_wallet_json")"
printf "Sponsor address: %s\n" "$SPONSOR_ADDR"

# Fund the sponsor address (sponsor pays gas)
fund_and_wait "$SPONSOR_ADDR"

echo -e "\n=== CAST SEND WITH SPONSOR (--tempo.sponsor-signature) ==="
# Test sponsored transactions using pre-signed signature.
# Step 1: Get the fee_payer_signature_hash using --tempo.print-sponsor-hash
# Step 2: Sign it with the sponsor's private key
# Step 3: Send with --tempo.sponsor-signature

# Step 1: Get the hash that the sponsor needs to sign
FEE_PAYER_HASH=$(cast mktx ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" \
  --tempo.print-sponsor-hash)
printf "Fee payer signature hash: %s\n" "$FEE_PAYER_HASH"

# Step 2: Sponsor signs the hash
SPONSOR_SIG=$(cast wallet sign --private-key "$SPONSOR_PK" "$FEE_PAYER_HASH" --no-hash)
printf "Sponsor signature: %s\n" "$SPONSOR_SIG"

# Step 3: Send the sponsored transaction with the signature
RECEIPT=$(cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$PK" \
  --tempo.sponsor-signature "$SPONSOR_SIG" --json)

# Verify the fee_payer in the receipt matches the sponsor address
RECEIPT_FEE_PAYER=$(echo "$RECEIPT" | jq -r '.feePayer // .fee_payer // empty')
if [[ -z "$RECEIPT_FEE_PAYER" ]]; then
  echo "ERROR: feePayer not found in receipt"
  echo "Receipt: $RECEIPT"
  exit 1
fi

# Normalize addresses for comparison (lowercase)
RECEIPT_FEE_PAYER_LOWER=$(echo "$RECEIPT_FEE_PAYER" | tr '[:upper:]' '[:lower:]')
SPONSOR_ADDR_LOWER=$(echo "$SPONSOR_ADDR" | tr '[:upper:]' '[:lower:]')
if [[ "$RECEIPT_FEE_PAYER_LOWER" != "$SPONSOR_ADDR_LOWER" ]]; then
  echo "ERROR: Receipt feePayer ($RECEIPT_FEE_PAYER) does not match sponsor ($SPONSOR_ADDR)"
  exit 1
fi
echo "SUCCESS: Receipt feePayer ($RECEIPT_FEE_PAYER) matches sponsor address"

# Batch transaction tests (available on all hardforks)
echo -e "\n=== CAST BATCH-MKTX (NATIVE BATCHING) ==="
# Build a batch transaction with multiple calls as a single type 0x76 transaction
cast batch-mktx ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
  --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
  --private-key "$PK"

echo -e "\n=== CAST BATCH-SEND (NATIVE BATCHING) ==="
# Send a batch transaction with multiple calls as a single type 0x76 transaction
cast batch-send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
  --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
  --private-key "$PK"

echo -e "\n=== CAST BATCH-SEND WITH VALUE SYNTAX (NATIVE BATCHING) ==="
# Test batch transaction with value syntax (currently using 0 value)
# TODO: Update to use non-zero value (e.g., 0.0001ether) once tempo#2294 is merged
# and the node supports per-call value transfers in batch transactions.
cast batch-send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D:0:increment()" \
  --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
  --private-key "$PK"

echo -e "\n=== DEPLOY COUNTER WITH REQUIRE ==="
# Use CounterWithRequire.sol (has require(newNumber > 100)) for batch revert testing
cp "$SCRIPT_DIR/contracts/CounterWithRequire.sol" src/Counter.sol
forge build
REQUIRE_COUNTER_OUTPUT=$(forge create ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} src/Counter.sol:Counter --rpc-url "$ETH_RPC_URL" --private-key "$PK" --broadcast 2>&1)
echo "Deploy output: $REQUIRE_COUNTER_OUTPUT"
# Extract address from human-readable output (avoids jq parse errors from stderr log pollution)
REQUIRE_COUNTER=$(echo "$REQUIRE_COUNTER_OUTPUT" | sed -n 's/.*Deployed to: \(0x[a-fA-F0-9]*\).*/\1/p')
if [[ "$REQUIRE_COUNTER" == "null" || -z "$REQUIRE_COUNTER" ]]; then
  echo "ERROR: Failed to deploy Counter with require"
  exit 1
fi
echo "Counter with require deployed at: $REQUIRE_COUNTER"

echo -e "\n=== CAST BATCH-SEND REVERT TEST ==="
# Test that batch reverts atomically when one call fails
# setNumber(1) fails because require(newNumber > 100)
if cast batch-send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  --call "$REQUIRE_COUNTER::increment()" \
  --call "$REQUIRE_COUNTER::setNumber(uint256):1" \
  --private-key "$PK" 2>&1; then
  echo "ERROR: Batch should have reverted but succeeded"
  exit 1
fi
echo "OK: Batch correctly reverted (setNumber(1) failed require > 100)"

# TODO(upstream): re-enable once cast batch-send supports pre-encoded calldata
# Currently fails with gas estimation revert
# echo -e "\n=== CAST BATCH-SEND WITH ARGS AND ENCODED CALLDATA ==="
# Test batch with both function arguments and pre-encoded calldata
# First call: pre-encoded calldata for setNumber(200)
# Second call: function signature with args setNumber(101)
# Final number should be 101 (second call executes last)
# ENCODED_CALLDATA=$(cast calldata "setNumber(uint256)" 200)
# echo "Encoded calldata for setNumber(200): $ENCODED_CALLDATA"
# cast batch-send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
#   --call "$REQUIRE_COUNTER::$ENCODED_CALLDATA" \
#   --call "$REQUIRE_COUNTER::setNumber(uint256):101" \
#   --private-key "$PK"

# NUMBER=$(cast call --rpc-url "$ETH_RPC_URL" "$REQUIRE_COUNTER" "number()(uint256)")
# echo "Counter number after batch: $NUMBER (expected: 101)"

# TODO(upstream): re-enable forge script --batch tests once fee token validation is fixed
# Currently fails with "invalid fee token: 0x0000000000000000000000000000000000000000"
# echo -e "\n=== FORGE SCRIPT --BATCH (NATIVE BATCHING) ==="
# Create a script that calls multiple contracts and batch them into a single tx
# Use template file and substitute REQUIRE_COUNTER address
# sed "s/\${REQUIRE_COUNTER}/${REQUIRE_COUNTER}/" "$SCRIPT_DIR/contracts/BatchTest.s.sol.template" > script/BatchTest.s.sol

# Get number before batch
# NUMBER_BEFORE=$(cast call --rpc-url "$ETH_RPC_URL" "$REQUIRE_COUNTER" "number()(uint256)")
# echo "Counter number before forge script --batch: $NUMBER_BEFORE"

# Run forge script with --batch flag
# forge script script/BatchTest.s.sol --broadcast --batch ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" --private-key "$PK"

# Verify all calls executed atomically
# NUMBER_AFTER=$(cast call --rpc-url "$ETH_RPC_URL" "$REQUIRE_COUNTER" "number()(uint256)")
# echo "Counter number after forge script --batch: $NUMBER_AFTER (expected: 503)"
# if [[ "$NUMBER_AFTER" != "503" ]]; then
#   echo "ERROR: Expected number to be 503 (500 + 3 increments), got $NUMBER_AFTER"
#   exit 1
# fi
# echo "OK: forge script --batch executed all calls atomically"

# echo -e "\n=== FORGE SCRIPT --BATCH WITH DEPLOY + CALLS ==="
# Test deploying a contract and calling it in the same batch transaction
# This tests the CREATE + CALL pattern (CREATE must be first)
# cp "$SCRIPT_DIR/contracts/BatchCounter.sol" src/BatchCounter.sol
# cp "$SCRIPT_DIR/contracts/DeployAndCall.s.sol" script/DeployAndCall.s.sol

# forge build

# Build verification args if VERIFIER_URL is set (same pattern as tempo-deploy.sh)
# VERIFY_ARG=()
# if [[ -n "${VERIFIER_URL:-}" ]]; then
#   VERIFY_ARG=(--verify --retries 10 --delay 10)
#   echo "Will verify deployed contract via $VERIFIER_URL"
# fi

# Run forge script with --batch flag - deploys and calls atomically
# forge script script/DeployAndCall.s.sol --broadcast --batch ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} ${VERIFY_ARG[@]+"${VERIFY_ARG[@]}"} --rpc-url "$ETH_RPC_URL" --private-key "$PK"

# echo "OK: forge script --batch with deploy + calls executed atomically"

# echo -e "\n=== FORGE SCRIPT --BATCH REVERT TEST ==="
# Test that batch reverts atomically when one call in the script fails
# Use template file and substitute REQUIRE_COUNTER address
# sed "s/\${REQUIRE_COUNTER}/${REQUIRE_COUNTER}/" "$SCRIPT_DIR/contracts/BatchRevertTest.s.sol.template" > script/BatchRevertTest.s.sol

# NUMBER_BEFORE_REVERT=$(cast call --rpc-url "$ETH_RPC_URL" "$REQUIRE_COUNTER" "number()(uint256)")
# echo "Counter number before batch revert test: $NUMBER_BEFORE_REVERT"

# if forge script script/BatchRevertTest.s.sol --broadcast --batch ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" --private-key "$PK" 2>&1; then
#   echo "ERROR: Batch script should have reverted but succeeded"
#   exit 1
# fi

# Verify number unchanged (atomic revert)
# NUMBER_AFTER_REVERT=$(cast call --rpc-url "$ETH_RPC_URL" "$REQUIRE_COUNTER" "number()(uint256)")
# echo "Counter number after batch revert: $NUMBER_AFTER_REVERT (expected: $NUMBER_BEFORE_REVERT - unchanged)"
# if [[ "$NUMBER_AFTER_REVERT" != "$NUMBER_BEFORE_REVERT" ]]; then
#   echo "ERROR: Expected number to remain $NUMBER_BEFORE_REVERT after atomic revert, got $NUMBER_AFTER_REVERT"
#   exit 1
# fi
# echo "OK: forge script --batch correctly reverted atomically"

echo -e "\n=== DEPLOY HIGH GAS CONTRACT ==="
# Deploy a contract that can burn ~15M gas via cold storage writes (mapping)
# Each cold SSTORE to a new slot costs ~22,000 gas; ~650 iterations ≈ 15M gas
cat > src/GasBurner.sol <<'SOLEOF'
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;
contract GasBurner {
    mapping(uint256 => uint256) public values;
    function burn(uint256 iterations) public {
        for (uint256 i; i < iterations; i++) {
            values[i] = i;
        }
    }
}
SOLEOF
forge build

GAS_BURNER_OUTPUT=$(forge create ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} src/GasBurner.sol:GasBurner --rpc-url "$ETH_RPC_URL" --private-key "$PK" --broadcast 2>&1)
echo "Deploy output: $GAS_BURNER_OUTPUT"
GAS_BURNER=$(echo "$GAS_BURNER_OUTPUT" | sed -n 's/.*Deployed to: \(0x[a-fA-F0-9]*\).*/\1/p')
GAS_BURNER_TX=$(echo "$GAS_BURNER_OUTPUT" | sed -n 's/.*Transaction hash: \(0x[a-fA-F0-9]*\).*/\1/p')
if [[ -z "$GAS_BURNER" ]]; then
  echo "ERROR: Failed to deploy GasBurner"
  exit 1
fi
echo "GasBurner deployed at: $GAS_BURNER (tx: $GAS_BURNER_TX)"

echo -e "\n=== CAST SEND HIGH GAS TX (~15M gas) ==="
GAS_BURN_RECEIPT=$(cast send ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} --rpc-url "$ETH_RPC_URL" \
  "$GAS_BURNER" 'burn(uint256)' 650 --gas-limit 15000000 --private-key "$PK" --json)
GAS_BURN_TX=$(echo "$GAS_BURN_RECEIPT" | jq -r '.transactionHash')
echo "High gas tx: $GAS_BURN_TX"
GAS_USED=$(echo "$GAS_BURN_RECEIPT" | jq -r '.gasUsed')
GAS_USED_DEC=$((GAS_USED))
echo "Gas used: $GAS_USED_DEC"

echo -e "\n=== DEPLOY LARGE CONTRACT ==="
# Deploy a contract at exactly the 24KB EIP-170 code size limit (24576 bytes deployed bytecode).
# 24174 bytes of padding + contract overhead = exactly 24576 bytes deployed.
PADDING=$(python3 -c "print('ff' * 24174)")
cat > src/MaxSizeContract.sol <<SOLEOF
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;
contract MaxSizeContract {
    bytes public constant PADDING = hex"${PADDING}";
    function ping() external pure returns (uint256) { return 1; }
}
SOLEOF
forge build

MAX_SIZE_OUTPUT=$(forge create ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} src/MaxSizeContract.sol:MaxSizeContract --rpc-url "$ETH_RPC_URL" --private-key "$PK" --broadcast 2>&1)
echo "Deploy output: $MAX_SIZE_OUTPUT"
MAX_SIZE_ADDR=$(echo "$MAX_SIZE_OUTPUT" | sed -n 's/.*Deployed to: \(0x[a-fA-F0-9]*\).*/\1/p')
if [[ -z "$MAX_SIZE_ADDR" ]]; then
  echo "ERROR: Failed to deploy MaxSizeContract"
  exit 1
fi
echo "MaxSizeContract deployed at: $MAX_SIZE_ADDR"

# Verify deployed code size hits the 24KB EIP-170 limit
# cast code returns hex string with 0x prefix; subtract 2 for prefix, divide by 2 for bytes
CODE_HEX=$(cast code --rpc-url "$ETH_RPC_URL" "$MAX_SIZE_ADDR")
CODE_SIZE=$(( (${#CODE_HEX} - 2) / 2 ))
echo "Deployed code size: $CODE_SIZE bytes (limit: 24576)"
if [[ $CODE_SIZE -ne 24576 ]]; then
  echo "ERROR: Deployed code size $CODE_SIZE != 24576 (expected exactly the EIP-170 limit)"
  exit 1
fi
echo "OK: Large contract deployed at exactly the EIP-170 limit ($CODE_SIZE bytes)"

# Verify the contract is callable
PING_RESULT=$(cast call --rpc-url "$ETH_RPC_URL" "$MAX_SIZE_ADDR" 'ping()(uint256)')
if [[ "$PING_RESULT" != "1" ]]; then
  echo "ERROR: ping() returned $PING_RESULT, expected 1"
  exit 1
fi
echo "OK: MaxSizeContract ping() returned 1"

# Skip DEX/liquidity tests when using custom fee token (they assume multiple fee tokens)
if [[ ${#FEE_TOKEN_ARG[@]} -eq 0 ]]; then
  echo -e "\n=== CHANGE USER DEFAULT FEE TOKEN ==="
  cast send --rpc-url "$ETH_RPC_URL" 0xfeec000000000000000000000000000000000000 'setUserToken(address)' 0x20C0000000000000000000000000000000000002 --private-key "$PK"
  cast send --rpc-url "$ETH_RPC_URL" 0xfeec000000000000000000000000000000000000 'setUserToken(address)' 0x20C0000000000000000000000000000000000000 --private-key "$PK"

  echo -e "\n=== ADD LIQUIDITY: APPROVE DEX ==="
  cast erc20 approve 0x20c0000000000000000000000000000000000002 0xdec0000000000000000000000000000000000000 10000000000 --rpc-url "$ETH_RPC_URL" --private-key "$PK"
  cast erc20 approve 0x20c0000000000000000000000000000000000000 0xdec0000000000000000000000000000000000000 10000000000 --rpc-url "$ETH_RPC_URL" --private-key "$PK"

  echo -e "\n=== ADD LIQUIDITY: PLACE BID ==="
  cast send 0xdec0000000000000000000000000000000000000 "place(address,uint128,bool,int16)" 0x20c0000000000000000000000000000000000002 100000000 true 10 --private-key "$PK" -r "$ETH_RPC_URL"

  echo -e "\n=== ADD LIQUIDITY: PLACE ASK ==="
  cast send 0xdec0000000000000000000000000000000000000 "place(address,uint128,bool,int16)" 0x20c0000000000000000000000000000000000002 100000000 false 10 --private-key "$PK" -r "$ETH_RPC_URL"

  echo -e "\n=== ADD LIQUIDITY: PLACE FLIP ==="
  cast send 0xdec0000000000000000000000000000000000000 "placeFlip(address,uint128,bool,int16,int16)" 0x20c0000000000000000000000000000000000002 100000000 true -10 10 --private-key "$PK" -r "$ETH_RPC_URL"

  echo -e "\n=== ADD LIQUIDITY: SWAP EXACT AMOUNT IN ==="
  cast send 0xdec0000000000000000000000000000000000000 "swapExactAmountIn(address,address,uint128,uint128)" 0x20c0000000000000000000000000000000000000 0x20c0000000000000000000000000000000000002 100000000 9000000 --private-key "$PK" -r "$ETH_RPC_URL"

# TODO(upstream): re-enable once the following error is fixed:
# Error: server returned an error response: error code -32000: replacement transaction underpriced
  # echo -e "\n=== ADD LIQUIDITY: SWAP EXACT AMOUNT OUT ==="
  # cast send 0xdec0000000000000000000000000000000000000 "swapExactAmountOut(address,address,uint128,uint128)" 0x20c0000000000000000000000000000000000002 0x20c0000000000000000000000000000000000000 9000000 100000000 --private-key "$PK" -r "$ETH_RPC_URL"
else
  echo -e "\n=== CHANGE USER DEFAULT FEE TOKEN ==="
  echo "skipped (custom fee token set)"

  echo -e "\n=== ADD LIQUIDITY: APPROVE DEX ==="
  echo "skipped (custom fee token set)"

  echo -e "\n=== ADD LIQUIDITY: PLACE BID ==="
  echo "skipped (custom fee token set)"

  echo -e "\n=== ADD LIQUIDITY: PLACE ASK ==="
  echo "skipped (custom fee token set)"

  echo -e "\n=== ADD LIQUIDITY: PLACE FLIP ==="
  echo "skipped (custom fee token set)"

  echo -e "\n=== ADD LIQUIDITY: SWAP EXACT AMOUNT IN ==="
  echo "skipped (custom fee token set)"

  echo -e "\n=== ADD LIQUIDITY: SWAP EXACT AMOUNT OUT ==="
  echo "skipped (custom fee token set)"
fi

# TODO(upstream): re-enable anvil local/fork tests once T3 hardfork is supported by anvil
# Currently fails with "Unknown hardfork: t3"
# echo -e "\n=== ANVIL LOCAL TESTS ==="

# ANVIL_PORT=8546
# echo "Starting local anvil..."
# Pass hardfork to anvil (lowercase for CLI compatibility)
# ANVIL_HARDFORK=$(echo "$HARDFORK" | tr '[:upper:]' '[:lower:]')
# anvil --tempo --hardfork "$ANVIL_HARDFORK" --port $ANVIL_PORT &
# ANVIL_PID=$!

# Ensure anvil is stopped on script exit
# trap 'kill "$ANVIL_PID" 2>/dev/null || true' EXIT

# Wait for anvil to be ready (max 10 seconds)
# for i in {1..10}; do
#   if cast client --rpc-url "http://127.0.0.1:$ANVIL_PORT" 2>/dev/null; then
#     echo "Anvil fork started successfully"
#     break
#   fi
#   if [[ $i -eq 10 ]]; then
#     echo "ERROR: Anvil fork failed to start"
#     exit 1
#   fi
#   sleep 1
# done

# ALICE_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

# echo -e "\n=== ANVIL LOCAL: CHECK CLIENT VERSION ==="
# cast client --rpc-url http://127.0.0.1:$ANVIL_PORT

# echo -e "\n=== ANVIL LOCAL: CHECK CHAIN ID ==="
# cast chain-id --rpc-url http://127.0.0.1:$ANVIL_PORT

# echo -e "\n=== ANVIL LOCAL: FORGE TEST ==="
# TEMPO_FEE_TOKEN="$FEE_TOKEN" forge test --rpc-url http://127.0.0.1:$ANVIL_PORT

# echo -e "\n=== ANVIL LOCAL: FORGE SCRIPT SIMULATE ==="
# TEMPO_FEE_TOKEN="$FEE_TOKEN" forge script ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} script/Mail.s.sol --sig "run(string)" "$(date +%s%N)" --rpc-url http://127.0.0.1:$ANVIL_PORT --private-key "$ALICE_PK"

# echo -e "\n=== ANVIL LOCAL: FORGE SCRIPT BROADCAST ==="
# TEMPO_FEE_TOKEN="$FEE_TOKEN" forge script ${FEE_TOKEN_ARG[@]+"${FEE_TOKEN_ARG[@]}"} script/Mail.s.sol --sig "run(string)" "$(date +%s%N)" --rpc-url http://127.0.0.1:$ANVIL_PORT --private-key "$ALICE_PK" --broadcast

# echo -e "\n=== ANVIL LOCAL: CAST SEND ==="
# cast send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$ALICE_PK"

# echo -e "\n=== ANVIL LOCAL: ERC20 TRANSFER ==="
# cast erc20 transfer --tempo.fee-token "$FEE_TOKEN" 0x20c0000000000000000000000000000000000000 0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 123456 --rpc-url http://127.0.0.1:$ANVIL_PORT --private-key "$ALICE_PK"

# echo -e "\n=== ANVIL LOCAL: CAST SEND WITH NONCE-KEY (2D Nonce) ==="
# cast send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$ALICE_PK" --nonce 0 --tempo.nonce-key 100

# echo -e "\n=== ANVIL LOCAL: CAST SEND WITH EXPIRING NONCE ==="
# cast send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$ALICE_PK" --tempo.expiring-nonce --tempo.valid-before "$(($(date +%s) + 25))"

# echo -e "\n=== ANVIL LOCAL: BATCH SEND ==="
# cast batch-send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT \
#   --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
#   --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
#   --private-key "$ALICE_PK"

# Stop anvil
# kill "$ANVIL_PID" 2>/dev/null || true
# trap - EXIT

# echo -e "\n=== ANVIL LOCAL TESTS COMPLETE ==="

# echo -e "\n=== ANVIL FORK TESTS ==="
# Use a fresh wallet for fork tests to avoid fee token exhaustion from prior devnet tests
# echo -e "\n=== ANVIL FORK: CREATE AND FUND FRESH WALLET ==="
# fork_wallet_json="$(cast wallet new --json)"
# FORK_ADDR="$(jq -r '.[0].address' <<<"$fork_wallet_json")"
# FORK_PK="$(jq -r '.[0].private_key' <<<"$fork_wallet_json")"
# printf "Fork test address: %s\n" "$FORK_ADDR"
# fund_and_wait "$FORK_ADDR"

# Set the fee token on devnet before forking so the fork snapshot includes it
# cast send --rpc-url "$ETH_RPC_URL" 0xfeec000000000000000000000000000000000000 \
#   'setUserToken(address)' "$FEE_TOKEN" --private-key "$FORK_PK"

# ANVIL_PORT=8547
# echo "Starting forked anvil..."
# Pass hardfork to anvil (lowercase for CLI compatibility)
# ANVIL_HARDFORK=$(echo "$HARDFORK" | tr '[:upper:]' '[:lower:]')
# anvil --tempo --hardfork "$ANVIL_HARDFORK" --fork-url "$ETH_RPC_URL" --port $ANVIL_PORT --retries 10 --timeout 60000 &
# ANVIL_PID=$!

# # Ensure anvil is stopped on script exit
# trap 'kill "$ANVIL_PID" 2>/dev/null || true' EXIT

# # Wait for anvil to be ready (max 10 seconds)
# for i in {1..10}; do
#   if cast client --rpc-url "http://127.0.0.1:$ANVIL_PORT" 2>/dev/null; then
#     echo "Anvil fork started successfully"
#     break
#   fi
#   if [[ $i -eq 10 ]]; then
#     echo "ERROR: Anvil fork failed to start"
#     exit 1
#   fi
#   sleep 1
# done

# echo -e "\n=== ANVIL FORK: CHECK CLIENT VERSION ==="
# cast client --rpc-url http://127.0.0.1:$ANVIL_PORT

# echo -e "\n=== ANVIL FORK: CHECK CHAIN ID ==="
# cast chain-id --rpc-url http://127.0.0.1:$ANVIL_PORT

# echo -e "\n=== ANVIL FORK: CHECK BLOCK NUMBER ==="
# cast block-number --rpc-url http://127.0.0.1:$ANVIL_PORT

# echo -e "\n=== ANVIL FORK: FORGE TEST ==="
# TEMPO_FEE_TOKEN="$FEE_TOKEN" forge test --rpc-url http://127.0.0.1:$ANVIL_PORT

# echo -e "\n=== ANVIL FORK: FORGE SCRIPT SIMULATE ==="
# TEMPO_FEE_TOKEN="$FEE_TOKEN" forge script --tempo.fee-token "$FEE_TOKEN" script/Mail.s.sol --sig "run(string)" "$(date +%s%N)" --rpc-url http://127.0.0.1:$ANVIL_PORT --private-key "$FORK_PK"

# echo -e "\n=== ANVIL FORK: FORGE SCRIPT BROADCAST ==="
# TEMPO_FEE_TOKEN="$FEE_TOKEN" forge script --tempo.fee-token "$FEE_TOKEN" script/Mail.s.sol --sig "run(string)" "$(date +%s%N)" --rpc-url http://127.0.0.1:$ANVIL_PORT --private-key "$FORK_PK" --broadcast

# echo -e "\n=== ANVIL FORK: CAST SEND ==="
# cast send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$FORK_PK"

# echo -e "\n=== ANVIL FORK: ERC20 TRANSFER ==="
# cast erc20 transfer --tempo.fee-token "$FEE_TOKEN" 0x20c0000000000000000000000000000000000000 0x4ef5DFf69C1514f4Dbf85aA4F9D95F804F64275F 123456 --rpc-url http://127.0.0.1:$ANVIL_PORT --private-key "$FORK_PK"

# echo -e "\n=== ANVIL FORK: CAST SEND WITH NONCE-KEY (2D Nonce) ==="
# cast send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$FORK_PK" --nonce 0 --tempo.nonce-key 100

# echo -e "\n=== ANVIL FORK: CAST SEND WITH EXPIRING NONCE ==="
# cast send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT 0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D 'increment()' --private-key "$FORK_PK" --tempo.expiring-nonce --tempo.valid-before "$(($(date +%s) + 25))"

# echo -e "\n=== ANVIL FORK: BATCH SEND ==="
# cast batch-send --tempo.fee-token "$FEE_TOKEN" --rpc-url http://127.0.0.1:$ANVIL_PORT \
#   --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
#   --call "0x86A2EE8FAf9A840F7a2c64CA3d51209F9A02081D::increment()" \
#   --private-key "$FORK_PK"

# Stop anvil
# kill "$ANVIL_PID" 2>/dev/null || true
# trap - EXIT

# echo -e "\n=== ANVIL FORK TESTS COMPLETE ==="

# echo -e "\n=== CHISEL FORK TESTS ==="
# Test chisel forking the Tempo network - precompiles should be accessible from fork

# Helper to check address has code via chisel fork
# check_has_code() {
#   local name="$1" addr="$2"
#   local result
#   result=$(chisel --fork-url "$ETH_RPC_URL" eval "address($addr).code.length > 0" 2>&1 | sed -n 's/.*Value: \(true\|false\).*/\1/p' || echo "")
#   if [[ "$result" != "true" ]]; then
#     echo "ERROR: $name ($addr) should have code when forking Tempo"
#     exit 1
#   fi
#   echo "OK: $name has code"
# }

# check_has_code "PathUSD" "0x20C0000000000000000000000000000000000000"
# check_has_code "AlphaUSD" "0x20C0000000000000000000000000000000000001"
# check_has_code "Nonce" "0x4e4F4E4345000000000000000000000000000000"
# check_has_code "AccountKeychain" "0xaAAAaaAA00000000000000000000000000000000"

# echo -e "\n=== CHISEL FORK TESTS COMPLETE ==="