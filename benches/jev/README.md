# Native Jev transaction generation

This branch adds an opt-in Rust implementation, independent of the earlier JavaScript experiments:

```sh
# OPENROUTER_API_KEY must already be configured. This opts in to remote ABI metadata disclosure.
forge test --invariant-tx-generator jev --invariant-workers 1
```

The equivalent setting is `tx_generator = "jev"` under `[profile.default.invariant]`.
The default is `"rng"`, which neither reads the credential nor starts a transport worker.

`TxGenerator` derives two productions for every eligible target/function pair: the function's
ABI-typed arguments generated randomly, or generated from Foundry's live dictionary. Jev chooses
the production, replacing RNG at that node. Argument values, senders, value, warp and roll remain
locally generated under existing constraints. This is an ABI grammar, not automatic inference of
semantic relationships such as `amount = balanceOf(sender)`.

Fresh transactions request Choice batches during fuzzing, not before compilation or via generated
Solidity. The next request includes up to eight actual execution observations: local function ID,
reverted, discarded, and new-coverage booleans. Corpus replay/mutation still runs locally; corpus
calls contribute feedback too. Target-generation changes invalidate pending decisions and re-derive
the grammar. EVM run resets clear history and pending decisions. Model output cannot introduce
new contracts, selectors, argument values, or executable code.

The fixed provider is OpenRouter's Decisions endpoint with model `typesafe/jev-1.13`. The opt-in
discloses eligible function signatures/types and local target ordinals, not contract addresses,
calldata, source, storage, or dictionary contents. The API key is only an authorization header and
is never included in configuration, requests, diagnostics, or recordings.

The experimental limits are eight choices per request, 32 requests per worker per invariant test
group, 64 eligible functions, 64 KiB requests, 256 KiB responses, and a two-second HTTP deadline.
The fuzz worker waits for each batch; this is not latency-free background guidance. Runtime and
cost budgets must include those waits. Missing credentials, provider errors, invalid batches,
oversized grammars or budget exhaustion disable further model calls for that worker and warn
before falling back to RNG. There are no automatic retries or redirects. Worker count multiplies
the per-worker request cap. Batch choices cannot anticipate future execution results.

Remote decisions are not seed-deterministic. Persisted concrete corpus/failure transactions remain
the replay artifact; rerunning only the RNG seed may produce different Jev decisions. This mode
does not currently persist provider responses or expose a deterministic decision replay stream.

The earlier JavaScript experiments remain in
[commit 55660b597](https://github.com/foundry-rs/foundry/tree/55660b597490a78b9bbc7c7217f2e739c1d52d78/benches/jev),
not in the active implementation. They are not measurements of this native mode. No native maze result is claimed
until the actual native binary has been built and run under matched end-to-end budgets.
