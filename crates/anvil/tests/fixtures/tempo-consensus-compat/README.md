# Native Tempo certificate verification

This test-only workspace member starts local Anvil nodes and decodes their HTTP
responses using Commonware 2026.9.0's
`Finalization<Scheme<PublicKey, MinSig>, Digest>` and `tempo_primitives::Block`.
It covers empty blocks, blocks containing Ethereum and Tempo transactions, and
T3/T4 epoch boundaries. It checks byte-identical re-encoding, header and body
commitments, and both signatures. Wrong identities and changes to the digest,
parent view, epoch, view or either signature must fail verification.

The package shares the root workspace lockfile and target directory. Its
one-way dependency on Anvil keeps `commonware-consensus` and its runtime out of
Anvil's production and test dependency graphs. It has only integration tests,
so release builds using `--bins` exclude it. Ordinary workspace tests and nextest
execute the checks on pull requests, including forks, without RPC secrets or
artifact files. The `tempo-consensus-compat` job in
`.github/workflows/ci-tempo.yml` also runs them nightly and on manual dispatch.

From the repository root:

```sh
cargo test --locked -p anvil-tempo-consensus-compat
```

Commonware's `sha256::Digest` only supplies the native fixed-width payload codec;
the actual header hash is Keccak-256 over the RLP-encoded `TempoHeader`.
