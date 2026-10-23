# Native Tempo certificate verification

This separate test workspace decodes actual Anvil HTTP responses using
Commonware 2026.9.0's `Finalization<Scheme<PublicKey, MinSig>, Digest>` and
`tempo_primitives::Block`. It checks byte-identical re-encoding, header and body
commitments, and both signatures. Wrong identities and changes to the digest,
parent view, epoch, view or either signature must fail verification.

The separate checked-in lockfile keeps `commonware-consensus` and its runtime out
of Anvil's production dependency graph and the root workspace lockfile. The
`tempo-consensus-compat` job in `.github/workflows/ci-tempo.yml` runs on ordinary
pull requests, including forks, with local nodes and no RPC secrets.

From the repository root:

```sh
export ANVIL_TEMPO_CONSENSUS_ARTIFACT=/tmp/tempo-consensus.json
cargo test --locked -p anvil --test it tempo::tempo_consensus_mined_certificate_matches_block_hash -- --exact
cargo test --locked -p anvil --test it tempo::tempo_consensus_nonempty_native_block_body -- --exact
cargo run --locked --manifest-path crates/anvil/tests/fixtures/tempo-consensus-compat/Cargo.toml
ANVIL_TEMPO_CONSENSUS_ARTIFACT=/tmp/tempo-consensus.nonempty.json \
  cargo run --locked --manifest-path crates/anvil/tests/fixtures/tempo-consensus-compat/Cargo.toml
```

The verifier also accepts the artifact on standard input when the environment
variable is unset. Input contains the complete `identity` and `finalization` RPC
results, including the native block. `certificate` is hex without a `0x` prefix;
`identity.identity` is the compressed 96-byte G2 public key with a `0x` prefix.
Commonware's `sha256::Digest` only supplies the native fixed-width payload codec;
the actual header hash is Keccak-256 over the RLP-encoded `TempoHeader`.
