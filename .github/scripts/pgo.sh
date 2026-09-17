#!/usr/bin/env bash
set -euo pipefail

: "${TARGET:?}"
: "${RUST_PROFILE:?}"
: "${OUT_DIR:?}"
: "${RUNNER_TEMP:?}"

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

pgo_root="$(mktemp -d "$RUNNER_TEMP/foundry-pgo.XXXXXX")"
anvil_pid=""
cleanup() {
  if [[ -n "$anvil_pid" ]] && kill -0 "$anvil_pid" 2>/dev/null; then
    kill -INT "$anvil_pid"
    wait "$anvil_pid" || true
  fi
  rm -rf "$pgo_root"
}
trap cleanup EXIT

corpus="$pgo_root/corpus"
profiles_dir="$pgo_root/profiles"
pgo_target="$pgo_root/target"
pgo_home="$pgo_root/home"
mkdir -p "$corpus/default" "$profiles_dir" "$OUT_DIR"
cp -R testdata/default/{core,inline,cheats} "$corpus/default/"
cp -R testdata/utils "$corpus/"

cat > "$corpus/foundry.toml" <<'EOF'
[profile.default]
test = "."
optimizer = true
solc = "0.8.36"
offline = true

[profile.default.invariant]
depth = 32

[fmt]
ignore = ["utils/Vm.sol"]
EOF

cat > "$corpus/solar-input.json" <<'EOF'
{
  "language": "Solidity",
  "sources": {
    "Counter.sol": {
      "content": "pragma solidity ^0.8.0; contract Counter { uint256 public number; function setNumber(uint256 value) public { number = value; } }"
    }
  },
  "settings": {
    "outputSelection": {"*": {"*": ["abi"]}}
  }
}
EOF

build_paths=(
  default/{core,inline}
  default/cheats/{Assert,Broadcast,Ec,ExpectEmit,Json,MappingStorageHooks,MockCalls}.t.sol
  default/cheats/{MonadReserveBalance,MonadStaking,Parse,Prank,Rlp,Sign,Wallet}.t.sol
)
mapfile -t solar_sources < <(
  find "$corpus/default/core" "$corpus/default/inline" -type f -name '*.sol' -print
  for path in "${build_paths[@]:2}"; do
    printf '%s\n' "$corpus/$path"
  done
)

solc="$pgo_home/.svm/0.8.36/solc-0.8.36"
mkdir -p "$(dirname "$solc")"
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$solc" \
  https://binaries.soliditylang.org/linux-amd64/solc-linux-amd64-v0.8.36+commit.8a079791
printf '%s  %s\n' c8d35afdddc3cd2743ee88b8f25e0fecd16e2bdd5f2120f37e52cd9cc45ae0e6 \
  "$solc" | sha256sum --check
chmod +x "$solc"

bins=(anvil cast chisel forge solar)
bin_args=(--workspace)
for name in "${bins[@]}"; do
  bin_args+=(--bin "$name")
done

export CFLAGS="${CFLAGS:+$CFLAGS }-fno-profile-generate -fno-profile-use"
export CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }-fno-profile-generate -fno-profile-use"
# Training uses worker threads, so update LLVM profile counters atomically.
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$pgo_target" \
  RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-Cprofile-generate=$profiles_dir \
    -Cllvm-args=-instrprof-atomic-counter-update-all" \
  cargo build "$@" "${bin_args[@]}"

bin_dir="$pgo_target/$TARGET/$RUST_PROFILE"
profile() {
  local name="$1"
  shift
  LLVM_PROFILE_FILE="$profiles_dir/$name-%m-%p.profraw" HOME="$pgo_home" "$@"
}

seed=0x00000000000000000000000000000000000000000000000000000000feedface
forge="$bin_dir/forge"
# Train cold and warm builds, unit tests, fuzzing, invariants, and formatting.
profile forge "$forge" build --root "$corpus" --no-lint -q "${build_paths[@]}"
profile forge "$forge" build --root "$corpus" --no-lint -q "${build_paths[@]}"
profile forge "$forge" test --root "$corpus" default/cheats/ExpectEmit.t.sol \
  --fuzz-seed "$seed" --threads 4 -q
profile forge "$forge" test --root "$corpus" default/cheats/Parse.t.sol \
  --fuzz-seed "$seed" --fuzz-runs 4096 --threads 4 -q
profile forge "$forge" test --root "$corpus" default/inline/InvariantInlineConf.t.sol \
  --fuzz-seed "$seed" --threads 4 -q
profile forge "$forge" fmt --root "$corpus" --check

cast="$bin_dir/cast"
address=0x0000000000000000000000000000000000000001
calldata="$(profile cast "$cast" calldata 'transfer(address,uint256)' "$address" 42)"
profile cast "$cast" abi-encode 'transfer(address,uint256)' "$address" 42 >/dev/null
profile cast "$cast" decode-calldata 'transfer(address,uint256)' "$calldata" >/dev/null
profile cast "$cast" keccak 'Foundry profile-guided optimization' >/dev/null
profile cast "$cast" wallet new-mnemonic --accounts 4 \
  --entropy 0xdf9bf37e6fcdf9bf37e6fcdf9bf37e3c --json >/dev/null

chisel="$bin_dir/chisel"
profile chisel "$chisel" --offline eval 'type(uint256).max / 7' >/dev/null
profile chisel "$chisel" --offline eval 'keccak256(abi.encode(uint256(42), "foundry"))' \
  >/dev/null

solar="$bin_dir/solar"
profile solar "$solar" --base-path "$corpus" --stop-after parsing "${solar_sources[@]}"
profile solar "$solar" --base-path "$corpus" --stop-after analysis "${solar_sources[@]}"
profile solar "$solar" --standard-json "$corpus/solar-input.json" >/dev/null

anvil="$bin_dir/anvil"
anvil_log="$pgo_root/anvil.log"
LLVM_PROFILE_FILE="$profiles_dir/anvil-%m-%p.profraw" HOME="$pgo_home" \
  "$anvil" --port 18545 --silent >"$anvil_log" 2>&1 &
anvil_pid=$!
rpc_url=http://127.0.0.1:18545
ready=false
for ((attempt = 0; attempt < 100; attempt++)); do
  if curl -fsS -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
    "$rpc_url" >/dev/null; then
    ready=true
    break
  fi
  if ! kill -0 "$anvil_pid" 2>/dev/null; then
    break
  fi
  sleep 0.1
done
if [[ "$ready" != true ]]; then
  cat "$anvil_log"
  exit 1
fi

rpc() {
  curl -fsS -H 'content-type: application/json' --data "$1" "$rpc_url" >/dev/null
}
for ((id = 1; id <= 64; id++)); do
  rpc "{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"eth_getBlockByNumber\",\"params\":[\"latest\",false]}"
  rpc "{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"eth_getBalance\",\"params\":[\"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266\",\"latest\"]}"
  rpc "{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"eth_estimateGas\",\"params\":[{\"from\":\"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266\",\"to\":\"0x70997970c51812dc3a010c7d01b50e0d17dc79c8\",\"value\":\"0x1\"}]}"
done
for ((id = 1; id <= 8; id++)); do
  rpc "{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"eth_sendTransaction\",\"params\":[{\"from\":\"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266\",\"to\":\"0x70997970c51812dc3a010c7d01b50e0d17dc79c8\",\"value\":\"0x1\"}]}"
done
profile cast "$cast" block latest --rpc-url "$rpc_url" >/dev/null
profile cast "$cast" balance "$address" --rpc-url "$rpc_url" >/dev/null
kill -INT "$anvil_pid"
wait "$anvil_pid"
anvil_pid=""

shopt -s nullglob
profiles=("$profiles_dir"/*.profraw)
for name in "${bins[@]}"; do
  binary_profiles=("$profiles_dir/$name-"*.profraw)
  if ((${#binary_profiles[@]} == 0)); then
    echo "No $name profiles were generated"
    exit 1
  fi
done
if ((${#profiles[@]} == 0)) || find "${profiles[@]}" -size 0 | grep -q .; then
  echo "Incomplete profiling data"
  exit 1
fi

llvm_profdata="$(dirname "$(rustc --print target-libdir)")/bin/llvm-profdata"
"$llvm_profdata" merge -o "$profiles_dir/foundry.profdata" "${profiles[@]}"
rm -rf "$pgo_target"

CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$pgo_target" \
  RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-Cprofile-use=$profiles_dir/foundry.profdata" \
  cargo build "$@" "${bin_args[@]}"
for name in "${bins[@]}"; do
  cp "$bin_dir/$name" "$OUT_DIR/$name"
done

HOME="$pgo_home" "$OUT_DIR/forge" test --root "$corpus" \
  default/cheats/ExpectEmit.t.sol --fuzz-seed "$seed" --threads 4 -q
