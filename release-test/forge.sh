# docker run --rm -it -v $PWD/test:/test -w /test foundry sh

REPOSITORY="test"

> forge.txt

clear

rm -rf $REPOSITORY && mkdir $REPOSITORY

docker_run() {
  echo "\033[0;32m> $@\033[0m"

  echo "> $@" >> forge.txt

  cache=$(mktemp)

  docker run --rm -v "$PWD/$REPOSITORY":/"$REPOSITORY" -w /"$REPOSITORY" foundry "$@" > "$cache" 2>&1

  status=$?

  cat "$cache" | tee -a forge.txt

  rm -f "$cache"

  if [ $status -ne 0 ]; then
    echo "\033[0;31mERROR: Command failed with exit status $status: $@\033[0m"

    exit 1
  fi
}

# wallet=$(docker_run cast wallet new)
# echo "$wallet"
# Successfully created new keypair.
# Address:     0x2a187c63c5c5212006cBB5D42CCd0BF0F67B142E
# Private key: 0xacef3f4d5f7c6666e927c24af52f35c45c07990d1f199cd476b0189d1029419f

ADDRESS="0x2a187c63c5c5212006cBB5D42CCd0BF0F67B142E" # $(echo "$wallet" | grep -oE 'Address: *0x[a-fA-F0-9]+' | awk '{print $2}')
PRIVATE_KEY="0xacef3f4d5f7c6666e927c24af52f35c45c07990d1f199cd476b0189d1029419f"  # $(echo "$wallet" | grep -oE 'Private key: *0x[a-fA-F0-9]+' | awk '{print $3}')

# RPC_URL="https://westend-asset-hub-eth-rpc.polkadot.io"
RPC_URL="https://testnet-passet-hub-eth-rpc.polkadot.io"

# https://faucet.polkadot.io/
# docker_run cast balance $ADDRESS \
#   --rpc-url $RPC_URL

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

docker_run forge bind --resolc
docker_run forge bind-json --resolc
docker_run forge build --resolc
docker_run forge cache clean
docker_run forge cache ls
docker_run forge clean
# forge clone
docker_run forge compiler resolve --resolc
# docker_run forge completions zsh
docker_run forge config
# docker_run forge coverage --resolc || true # EXPECTED TO FAIL
docker_run forge create Counter --resolc \
  --rpc-url $RPC_URL \
  --private-key $PRIVATE_KEY \
  --broadcast \
  --constructor-args 0
docker_run forge doc
# forge eip712
docker_run forge flatten src/Counter.sol
docker_run forge fmt
docker_run forge geiger src/Counter.sol
docker_run forge generate test \
  --contract-name Counter
docker_run forge generate-fig-spec
docker_run forge inspect Counter bytecode --resolc
docker_run forge install vectorized/solady && docker_run forge update vectorized/solady
docker_run forge remappings
docker_run forge remove solady --force
# forge script ./test/Counter.t.sol --resolc \
#   --rpc-url $RPC_URL \
#   --sig "setUp()" \
#   --broadcast
# forge selectors collision
docker_run forge selectors upload --all
docker_run forge selectors list
docker_run forge selectors find 0xd09de08a
docker_run forge selectors cache
# docker_run forge snapshot --resolc || true # EXPECTED TO FAIL
docker_run forge soldeer init \
  --config-location foundry
docker_run forge soldeer install @openzeppelin-contracts~5.0.2
docker_run forge soldeer update
# forge soldeer login
# forge soldeer push
docker_run forge soldeer uninstall @openzeppelin-contracts
docker_run forge soldeer version
# docker_run forge test --resolc || true # EXPECTED TO FAIL
docker_run forge tree
# forge verify-bytecode
# forge verify-check
# forge verify-contract
