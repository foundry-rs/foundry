#!/bin/bash
for address in $@; do
    echo "Generating test data for $address";
    cast code --rpc-url "https://cloudflare-eth.com" $address | tee "testdata/${address}_encoded.txt" | evmasm -d >> "testdata/${address}_decoded.txt";
done

