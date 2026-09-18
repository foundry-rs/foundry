# Native Jev transaction generation

This branch adds an opt-in Rust implementation, independent of the earlier JavaScript experiments:

```sh
# OPENROUTER_API_KEY must already be configured. This opts in to remote ABI metadata disclosure.
forge test --invariant-tx-generator jev --invariant-workers 1
```

The equivalent setting is `tx_generator = "jev"` under `[profile.default.invariant]`.
The default is `"rng"`, which neither reads the credential nor starts a transport worker.

`TxGenerator` keeps RNG as the primary strategy and samples Jev for 20% of fresh transactions. For
each eligible target/function pair, the derived grammar offers random ABI arguments, Foundry
dictionary arguments, and compatible same-contract view results for one argument. A view candidate
must have exactly one output whose canonical ABI type equals the destination argument; it may take
no inputs or address inputs, which are bound to the selected sender. The remaining arguments stay
random. This treats `amount = balanceOf(sender)` as a model-selectable candidate relationship, not
as a claim that matching ABI types prove semantic validity.

Selected views execute locally against the current invariant-run state immediately before their
transaction. The read does not commit state. Successful results replace one argument and the final
concrete calldata is retained for corpus replay, shrinking, and failure reporting. A reverted,
halted, malformed, or undecodable view leaves the original random argument unchanged. View return
values, concrete arguments, senders, calldata, and storage never leave the process.

Fresh transactions request Choice batches during fuzzing, not before compilation or via generated
Solidity. The next request includes up to eight actual execution observations: local production ID,
reverted, discarded, and new-coverage booleans. Corpus replay/mutation still runs locally; corpus
calls contribute feedback too. Target-generation changes invalidate pending decisions and re-derive
the grammar. EVM run resets clear history and pending decisions. Model output cannot introduce
new contracts, selectors, argument values, or executable code.

The fixed provider is OpenRouter's Decisions endpoint with model `typesafe/jev-1.13`. The opt-in
discloses eligible function and compatible view signatures/types plus local target ordinals, not contract addresses,
calldata, source, storage, or dictionary contents. The API key is only an authorization header and
is never included in configuration, requests, diagnostics, or recordings.

The experimental limits are eight choices per request, 32 requests per worker per invariant test
group, 64 eligible functions, 256 grammar productions, 64 KiB requests, 256 KiB responses, and a two-second HTTP deadline.
The fuzz worker waits for each batch; this is not latency-free background guidance. Runtime and
cost budgets must include those waits. Missing credentials, provider errors, invalid batches,
oversized grammars or budget exhaustion disable further model calls for that worker and warn
before falling back to RNG. There are no automatic retries or redirects. Worker count multiplies
the per-worker request cap. Batch choices cannot anticipate future execution results.

Remote decisions are not seed-deterministic. Persisted concrete corpus/failure transactions remain
the replay artifact; rerunning only the RNG seed may produce different Jev decisions. This mode
does not currently persist provider responses or expose a deterministic decision replay stream.

## Stateful semantic smoke benchmark

The fixture in [`fixtures`](fixtures) tests the relationship
`withdrawExact(withdrawableBalanceOf(sender))` after a deposit. The derived amount is neither a
stored dictionary leaf nor exposed as a useful comparison operand. Run it with:

```sh
forge clean --root benches/jev/fixtures
RUST_LOG=forge::jev=debug forge test --root benches/jev/fixtures \
  --fuzz-seed 0x585f37fbac9620027325a193e979e3c93c87b069b15dce6c903f5f460436f916 -vv
```

In a native live-provider run on 2026-09-18, the mixed Jev mode found the failure after 7 runs,
647 calls, and 20 request batches. After cleaning persisted failures, the same binary and seed with
`--invariant-tx-generator rng` completed 16 runs and 1,600 calls without finding it. This is a
targeted integration smoke test of the requested semantic relationship, not a general fuzzing
performance claim.

The earlier JavaScript experiments remain in
[commit 55660b597](https://github.com/foundry-rs/foundry/tree/55660b597490a78b9bbc7c7217f2e739c1d52d78/benches/jev),
not in the active implementation. They are not measurements of this native mode. No native maze result is claimed
until the actual native binary has been built and run under matched end-to-end budgets.
