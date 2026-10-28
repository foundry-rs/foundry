# Tempo certificate and storage-proof acceptance

This fixture consumes fresh local Anvil HTTP responses in Solidity: Commonware
verifies vote and seed BLS signatures through EIP-2537, then the adapter binds
the certificate to the native header and verifies the account and storage
Merkle-Patricia proofs against its state root with Optimism's libraries.

One node follows a complete timeline with an epoch length of four:

1. Deploy `ProofStore` at block 1 and write 42 at block 2. Collect its certificate,
   raw header and `eth_getProof` before scheduling rotation.
2. Schedule rotation and authenticate block 3's DKG announcement with the old key.
3. Extract the next key from that announcement and verify block 4's storage proof.
4. Read unrelated accounts and write 99 at block 5; both earlier proofs must stay
   unchanged.

The initial `anvil_tempoConsensusIdentity` response is the explicit trust anchor.
The successor key must match the authenticated announcement, including its
compressed encoding. Anvil uses a deterministic development identity; this is a
compatibility fixture limited to the pinned one-participant DKG format, not a
production light client. Python's `py-ecc` only decompresses RPC points for the
upstream verifier's coordinate interface; Solidity verifies the signatures.

The tests reject altered signatures, rounds, headers and proofs, wrong keys,
account, slot, height or value, and unauthenticated or malformed key handoffs.

## Run locally

Use Linux or macOS and Python 3.11 or newer. From the repository root:

```sh
cargo build --profile dev --locked --bin forge --bin anvil
python3 -m venv testdata/tempo-consensus/.venv
testdata/tempo-consensus/.venv/bin/python -m pip install -r testdata/tempo-consensus/requirements.txt
testdata/tempo-consensus/.venv/bin/python testdata/tempo-consensus/bootstrap.py
ANVIL_BIN="$PWD/target/debug/anvil" FORGE_BIN="$PWD/target/debug/forge" \
  testdata/tempo-consensus/.venv/bin/python testdata/tempo-consensus/run.py
```

The runner builds the contracts, collects fresh evidence and requires every
compiled Solidity test to succeed, including both positive acceptance cases.
It rejects overlapping runs because Forge's output/cache are shared. Failures
exit nonzero and nodes are cleaned up. Raw RPC evidence and Forge output are
saved as `fixtures/proofs.json` and `fixtures/forge.json`; CI retains them as
artifacts. No public RPC or credentials are required.

The node uses Tempo T14; Solc is pinned to 0.8.30 and verification uses Prague.
The [native compatibility tests](../../crates/anvil/tests/fixtures/tempo-consensus-compat/README.md)
separately cover the T3/T4 consensus-context boundary. `sources.lock.json` pins
Commonware Solidity and Optimism sources by revision and SHA-256, retaining their
licenses. `bootstrap.py --offline` checks the cache; the runner always uses this
mode. Install dependencies, sources and Solc before running offline.

Runner failure and cleanup regressions:

```sh
testdata/tempo-consensus/.venv/bin/python -m unittest discover -s testdata/tempo-consensus -p 'test_*.py'
```
