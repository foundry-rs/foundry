# docker run --rm -it -v $PWD/test:/test -w /test foundry sh

REPOSITORY="test"

> cast.txt

clear

rm -rf $REPOSITORY && mkdir $REPOSITORY

docker_run() {
  echo "\033[0;32m> $@\033[0m"

  echo "> $@" >> cast.txt

  cache=$(mktemp)

  docker run --rm -v "$PWD/$REPOSITORY":/"$REPOSITORY" -w /"$REPOSITORY" foundry "$@" > "$cache" 2>&1

  status=$?

  cat "$cache" | tee -a cast.txt

  rm -f "$cache"

  if [ $status -ne 0 ]; then
    echo "\033[0;31mERROR: Command failed with exit status $status: $@\033[0m"

    exit 1
  fi
}

ADDRESS="0x2FCDC5f0799ACCb67008aaB9D4AAA08994897997"  # based on the output of `cast wallet new`
PRIVATE_KEY="0xf88c374c84378042e20927119c4c8b6ed2d57508c9b8a4f05fe2868ab8f8b73e"  # based on the output of `cast wallet new`

# RPC_URL="https://westend-asset-hub-eth-rpc.polkadot.io"
RPC_URL="https://testnet-passet-hub-eth-rpc.polkadot.io"

docker_run cast --version
docker_run forge --version

docker_run forge init .

cat << 'EOF' > $REPOSITORY/foundry.toml
[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[profile.default.resolc]
resolc_compile = true
EOF

CONTRACT=$(docker_run forge create Counter --resolc \
  --rpc-url $RPC_URL \
  --private-key $PRIVATE_KEY \
  --broadcast \
  --constructor-args 0
)
echo "$CONTRACT"
CONTRACT_ADDRESS=$(echo "$CONTRACT" | grep 'Deployed to:' | awk '{print $3}')
TRANSACTION_HASH=$(echo "$CONTRACT" | grep 'Transaction hash:' | awk '{print $3}')

docker_run cast 4byte 0xd09de08a
# cast 4byte-calldata
docker_run cast 4byte-event 0xb68ce3d4f35f8b562c4caf11012045e29a80cc1082438f785646ec651416c8d6
docker_run cast abi-encode "increment()"
# cast access-list
docker_run cast address-zero
docker_run cast admin $CONTRACT_ADDRESS \
  --rpc-url $RPC_URL
docker_run cast age latest \
  --rpc-url $RPC_URL
# cast artifact
docker_run cast balance $ADDRESS \
  --rpc-url $RPC_URL
docker_run cast base-fee latest \
  --rpc-url $RPC_URL
# cast bind
docker_run cast block latest \
  --rpc-url $RPC_URL
docker_run cast block-number latest \
  --rpc-url $RPC_URL
docker_run cast call $CONTRACT_ADDRESS "increment()" \
  --rpc-url $RPC_URL
docker_run cast calldata "increment()"
docker_run cast chain \
  --rpc-url $RPC_URL
docker_run cast chain-id \
  --rpc-url $RPC_URL
docker_run cast client \
  --rpc-url $RPC_URL
docker_run cast code $CONTRACT_ADDRESS \
  --rpc-url $RPC_URL
# cast codehash
docker_run cast codesize $CONTRACT_ADDRESS \
  --rpc-url $RPC_URL
# docker_run cast completions zsh
docker_run cast compute-address $ADDRESS \
  --rpc-url $RPC_URL
docker_run cast concat-hex 0xa 0xb 0xc
# cast constructor-args
# docker_run cast create2
# cast creation-code
docker_run cast decode-abi "balanceOf(address)(uint256)" 0x000000000000000000000000000000000000000000000000000000000000000a
docker_run cast decode-calldata "transfer(address,uint256)" 0xa9059cbb000000000000000000000000e78388b4ce79068e89bf8aa7f218ef6b9ab0e9d0000000000000000000000000000000000000000000000000008a8e4b1a3d8000
docker_run cast decode-error 0x4e487b710000000000000000000000000000000000000000000000000000000000000011 \
  --sig "Panic(uint256)"
docker_run cast decode-event 0x000000000000000000000000000000000000000000000000000000000000002a \
  --sig "CounterChanged(int256)"
# cast disassemble
docker_run cast estimate \
  --rpc-url $RPC_URL \
  --from $ADDRESS \
  $CONTRACT_ADDRESS \
  "increment()"
docker_run cast find-block $(date +%s) \
  --rpc-url $RPC_URL
docker_run cast format-bytes32-string "increment"
docker_run cast format-units 1
docker_run cast from-bin
docker_run cast from-fixed-point 1 1
docker_run cast from-rlp 0x696e6372656d656e740000000000000000000000000000000000000000000000
docker_run cast from-utf8 "increment"
# cast from-wei
docker_run cast gas-price \
  --rpc-url $RPC_URL
docker_run cast generate-fig-spec
docker_run cast hash-message "increment"
docker_run cast hash-zero
docker_run cast implementation $CONTRACT_ADDRESS \
  --rpc-url $RPC_URL
docker_run cast index string "increment" 1
docker_run cast index-erc7201 1
# cast interface
docker_run cast keccak "increment"
docker_run cast logs \
  --rpc-url $RPC_URL \
  --address $CONTRACT_ADDRESS \
  --from-block 78016
# cast lookup-address
docker_run cast max-int
docker_run cast max-uint
docker_run cast min-int
RAW_TRANSACTION=$(docker_run cast mktx $CONTRACT_ADDRESS "increment()" \
  --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io \
  --private-key $PRIVATE_KEY \
  --from $ADDRESS \
  | grep -E '^0x[0-9a-fA-F]+$'
)
echo "$RAW_TRANSACTION"
docker_run cast decode-transaction $RAW_TRANSACTION
docker_run cast namehash "increment"
docker_run cast nonce $ADDRESS \
  --rpc-url $RPC_URL
docker_run cast parse-bytes32-address 0x000000000000000000000000000000000000000000000000000000000000000a
docker_run cast parse-bytes32-string 0x696e6372656d656e740000000000000000000000000000000000000000000000
docker_run cast parse-units 1
docker_run cast pretty-calldata 0xd09de08a
# cast proof
docker_run cast publish $RAW_TRANSACTION \
  --rpc-url $RPC_URL
docker_run cast receipt $TRANSACTION_HASH \
  --rpc-url $RPC_URL
# cast resolve-name
docker_run cast rpc "eth_getTransactionByHash" $TRANSACTION_HASH \
  --rpc-url $RPC_URL
# cast run
# cast selectors
docker_run cast send $CONTRACT_ADDRESS "increment()" \
  --rpc-url $RPC_URL \
  --private-key $PRIVATE_KEY
docker_run cast shl 1 1
docker_run cast shr 1 1
docker_run cast sig "increment()"
docker_run cast sig-event "increment()"
docker_run cast storage $CONTRACT_ADDRESS 0xc7728314374610455dba288d68795a0a1f4e297598fadddf5234bb036cb803cc \
  --rpc-url $RPC_URL
# cast storage-root
docker_run cast to-ascii "0x696e6372656d656e74"
docker_run cast to-base 1 hex
docker_run cast to-bytes32 1
docker_run cast to-check-sum-address $ADDRESS
docker_run cast to-dec 0xff
docker_run cast to-fixed-point 1 1
docker_run cast to-hex 1
docker_run cast to-hexdata 0x1
docker_run cast to-int256 1
docker_run cast to-rlp '["0xaa","0xbb","cc"]'
docker_run cast to-uint256 1
docker_run cast to-unit 1 ether
docker_run cast to-utf8 0x74657374
docker_run cast to-wei 1 ether
docker_run cast tx $TRANSACTION_HASH \
  --rpc-url $RPC_URL
# cast tx-pool
docker_run cast upload-signature "transfer(uint256)"
docker_run cast wallet new
docker_run cast wallet new-mnemonic
docker_run cast wallet address \
  --private-key $PRIVATE_KEY
