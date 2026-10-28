# Tempo certificate and storage-proof acceptance

This fixture verifies fresh local Anvil HTTP responses in Solidity. It deploys
`ProofStore`, writes slot zero, and retrieves a certificate, native raw Tempo
header and `eth_getProof` at the same fixed height. Commonware's Solidity
library verifies the vote and seed BLS signatures through EIP-2537 precompiles.
The adapter then checks the native header hash and follows its `stateRoot`
through Optimism's account and storage Merkle-Patricia proof verifier to the
expected value, 42.

| Scenario | Acceptance |
| --- | --- |
| Standalone | Deploy and write storage, authenticate its block, then verify the account and slot proof. |
| Rotation | Authenticate the epoch-boundary announcement with the old key, obtain the next key from its DKG outcome, and verify the next epoch's storage proof. |

The initial `anvil_tempoConsensusIdentity` response is the fixture's explicit
trust anchor. Anvil uses a deterministic development identity: these tests
establish wire and verifier compatibility, not production validator-set trust.
Rotation obtains its next trusted key from the authenticated boundary header;
it does not trust a new identity RPC response for each epoch. The adapter is
limited to the pinned one-participant, nonzero-counter DKG format and is test
code rather than a general-purpose light client.

Both scenarios start standalone local nodes. After collecting a proof of value
42, the collector reads an unrelated account and writes 99 to the tested slot;
the earlier fixed-height proof must remain unchanged.

Negative cases reject the wrong key, account, slot, height or value; altered
headers, digests, rounds, signatures and trie proofs; and an unauthenticated
rotation. The pinned Tempo mainnet key is also rejected for the local node.
Cryptography and trie verification run in Solidity; `py-ecc` only decompresses
the original RPC points for the upstream verifier's coordinate interface. It
neither signs replacement certificates nor verifies signatures for Solidity.

## Run locally

Use Linux or macOS, Python 3.11 or newer, and Anvil/Forge binaries built from this
checkout. From the repository root:

```sh
cargo build --profile dev --locked --bin forge --bin anvil
python3 -m venv testdata/tempo-consensus/.venv
testdata/tempo-consensus/.venv/bin/python -m pip install -r testdata/tempo-consensus/requirements.txt
testdata/tempo-consensus/.venv/bin/python testdata/tempo-consensus/bootstrap.py
ANVIL_BIN="$PWD/target/debug/anvil" FORGE_BIN="$PWD/target/debug/forge" \
  testdata/tempo-consensus/.venv/bin/python testdata/tempo-consensus/run.py
```

The final command runs both scenarios with fresh localhost nodes and no public
RPC or credentials. Use `--scenario standalone` or `--scenario rotation` for one
scenario; `--anvil` and `--forge` also accept explicit binary paths. The runner
rejects overlapping runs in the same fixture directory to preserve the active
run's evidence and results.

The collector explicitly selects `--hardfork tempo:T14`; the Solidity compiler
is pinned to 0.8.30 and the Forge verification EVM to Prague for EIP-2537. This
fixture tests that combination, not every Tempo revision. The native
[compatibility tests](../../crates/anvil/tests/fixtures/tempo-consensus-compat/README.md)
separately cover the T3/T4 consensus-context boundary.

## Follow the verification flow

The [collector](collect.py) starts each node, reads its initial development
identity, deploys `ProofStore`, and mines a transaction that writes 42 to slot
zero. It selects one fixed height after the write, using the first block of the
next epoch for rotation, and queries `consensus_getFinalization`,
`debug_getRawHeader` and `eth_getProof` for that same height. The generated fixture
retains the original RPC responses and deployment/write receipts alongside the
decompressed BLS coordinates.

The [Solidity tests](test/TempoProof.t.sol) pass this evidence to
[`TempoProofVerifier.verify`](src/TempoProofVerifier.sol), which verifies the
certificate, native header hash, account proof and storage proof. In the rotation
scenario, `verifyAfterHandoff` first authenticates the boundary announcement with
the old key and binds its announced next key and epoch to the consumed evidence.
The next key is obtained from that announcement, without trusting another
identity RPC response. Run that complete flow alone with:

```sh
ANVIL_BIN="$PWD/target/debug/anvil" FORGE_BIN="$PWD/target/debug/forge" \
  testdata/tempo-consensus/.venv/bin/python testdata/tempo-consensus/run.py --scenario rotation
```

## Reproducibility and CI

`bootstrap.py` fetches the immutable revisions and SHA-256 hashes listed in
`sources.lock.json`, retaining upstream licenses. It refuses a mismatched cached
file. `bootstrap.py --offline` verifies the cache without downloading; the
runner always uses that mode. Install Python dependencies, pinned sources and
Solc 0.8.30 before an offline run.

Commonware Solidity is pinned to
`456538f0e63a2d8e4ee309af27af836f9466d5a1`, separately from the Commonware 2026.9.0
Rust release, which did not contain `sol/`. Optimism's trie libraries are pinned
to `b84018de4fc5ea47f13b6ebdf7a2f793d8334ea8`. Python dependency versions are in
`requirements.txt`.

Generated RPC evidence lives in `fixtures/{standalone,rotation}.json`.
`fixtures/results.json` records the run status, binary/source/fixture hashes and
verification output, including failures. Generated dependencies, compiler
artifacts and fixtures are excluded from Git. The `tempo-consensus-solidity`
job in `.github/workflows/ci-tempo.yml` builds the candidate binaries and runs
these same scenarios on pull requests, including forks, without RPC secrets.
CI retains the generated fixtures and results as an artifact.

Run the runner failure and process-cleanup regression tests with:

```sh
testdata/tempo-consensus/.venv/bin/python -m unittest discover -s testdata/tempo-consensus -p 'test_*.py'
```
