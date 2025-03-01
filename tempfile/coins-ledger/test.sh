#!/bin/sh

wasm-pack build --scope summa-tx --target nodejs -- --features=node --no-default-features && \
cd node_tests && \
rm -rf ./coins_ledger && \
cp -r ../pkg ./coins_ledger && \
npm i ./coins_ledger && \
npm run test
