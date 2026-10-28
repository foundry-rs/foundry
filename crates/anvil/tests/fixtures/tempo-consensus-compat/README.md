# Native Tempo certificate verification

This test-only workspace member starts local Anvil nodes and decodes their HTTP
responses using Commonware 2026.9.0's
`Finalization<Scheme<PublicKey, MinSig>, Digest>` and `tempo_primitives::Block`.
It covers empty blocks, blocks containing Ethereum and Tempo transactions, and
T3/T4 epoch boundaries. It checks byte-identical re-encoding, header and body
commitments, and both signatures. Wrong identities and changes to the digest,
parent view, epoch, view or either signature must fail verification. Raw header
bytes, Ethereum block hashes and consensus digests must identify the same block.

Rotation checks begin with one trusted identity, authenticate the boundary block,
and decode its DKG outcome to obtain the next epoch's key. The next certificate
must verify under that announced key and reject the old key. Snapshot replay
must restore the same announcement and activation without trusting a fresh
identity RPC response for each block.

The package shares the root workspace lockfile and target directory. Its
one-way dependency on Anvil keeps `commonware-consensus` and its runtime out of
Anvil's production and test dependency graphs. It has only integration tests,
so release builds using `--bins` exclude it. Ordinary workspace tests and nextest
execute the checks on pull requests, including forks, without RPC secrets or
artifact files. The `tempo-consensus-compat` job in
`.github/workflows/ci-tempo.yml` also runs them nightly and on manual dispatch.

These tests run only on Unix platforms because Commonware's native runtime does
not support Windows. On non-Unix targets, both the `commonware-consensus`
dependency and the native verification code are excluded from compilation.

From the repository root:

```sh
cargo test --locked -p anvil-tempo-consensus-compat
```

Commonware's `sha256::Digest` only supplies the native fixed-width payload codec;
the actual header hash is Keccak-256 over the RLP-encoded `TempoHeader`.

The [Solidity proof integration test](../../../../../docs/dev/networks.md#tempo-certificate-and-storage-proofs)
complements these native checks: it authenticates fresh certificates and native
headers, then verifies account and storage proofs in the EVM, including an
authenticated key rotation. It runs in the ordinary Forge CLI nextest suite
without public RPC access.
