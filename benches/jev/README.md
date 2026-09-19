# Native Jev transaction generation

This branch adds an opt-in Rust implementation and a replayable generated-Solidity prototype:

```sh
# OPENROUTER_API_KEY must already be configured. This opts in to remote ABI metadata disclosure.
forge test --invariant-tx-generator jev --invariant-workers 1
```

The equivalent setting is `tx_generator = "jev"` under `[profile.default.invariant]`.
The default is `"rng"`, which neither reads the credential nor starts a transport worker.

`TxGenerator` keeps RNG as the primary strategy and samples Jev for 20% of fresh transactions. For
each eligible target/function pair, the derived grammar offers random ABI arguments, Foundry
dictionary arguments, and compatible targeted-contract view results for one argument. A view
candidate must have exactly one output whose canonical ABI type equals the destination argument; it may take
no inputs or address inputs, which are bound to the selected sender. The remaining arguments stay
random. This treats `amount = balanceOf(sender)` as a model-selectable candidate relationship, not
as a claim that matching ABI types prove semantic validity.

The Jev path also acts as an automatically derived invariant handler. It derives two-action
lifecycle candidates from each contract ABI and relationship candidates from compatible getters.
Jev chooses a scenario plus a role pattern over three persistent local identities (primary actor,
counterparty, and third party), so ownership, approvals, balances, and obligations can carry across
or deliberately change between calls without a hand-written Solidity handler. Jev sees only role
names; Foundry owns the concrete addresses. A single-action ABI falls back to an eight-choice batch.

Selected views execute locally against the current invariant-run state immediately before their
transaction. The read does not commit state. Successful results replace one argument and the final
concrete calldata is retained for corpus replay, shrinking, and failure reporting. A reverted,
halted, malformed, or undecodable view leaves the original random argument unchanged. View return
values, concrete arguments, senders, calldata, and storage never leave the process.

Fresh transactions request Choice batches during fuzzing, not before compilation or via generated
Solidity. The next request includes up to eight actual execution observations: local production ID,
actor role, reverted, discarded, and new-coverage booleans. Corpus replay/mutation still runs
locally; corpus calls contribute feedback too. Target-generation changes invalidate pending decisions and re-derive
the grammar. EVM run resets clear history and pending decisions while preserving actor identities.
Model output cannot introduce new contracts, selectors, argument values, or executable code.

The fixed provider is OpenRouter's Decisions endpoint with model `typesafe/jev-1.13`. The opt-in
discloses eligible function and compatible view signatures/types plus local target ordinals, not
contract addresses, calldata, source, storage, or dictionary contents. The API key is only an authorization header and
is never included in configuration, requests, diagnostics, or recordings.

The experimental limits are either one derived scenario or eight action/actor pairs per request,
32 requests per worker per invariant test group, 128 eligible functions, 512 local productions,
256 model-visible choices, 256 KiB requests and responses, and a two-second HTTP deadline.
The fuzz worker waits for each batch; this is not latency-free background guidance. Runtime and
cost budgets must include those waits. Missing credentials, provider errors, invalid batches,
oversized grammars or budget exhaustion disable further model calls for that worker and warn
before falling back to RNG. There are no automatic retries or redirects. Worker count multiplies
the per-worker request cap. Batch choices cannot anticipate future execution results.

Remote decisions are not seed-deterministic. Persisted concrete corpus/failure transactions remain
the replay artifact; rerunning only the RNG seed may produce different Jev decisions. This mode
does not currently persist provider responses or expose a deterministic decision replay stream.

## Generated Solidity shortcuts

[`grammar`](grammar) is the complementary code-generation path. Jev selects bounded, typed action,
sender, and live-view productions; a local validated AST renderer materializes Solidity shortcut
handlers. Forge compiles and calls those handlers alongside ordinary actions. State reads remain at
each generated call site, so later steps observe changes made by their prerequisites. The checked-in
decision recording makes the generated source and execution reproducible without another provider
call or credential.

This demonstrates the compile/deploy/call mechanics on a local fund-flow ledger. The native path
above derives scenario candidates from arbitrary invariant ABIs, while the generated-Solidity
prototype still uses a fixed local model. Neither path presently infers a complete asset-flow graph
from Solidity source or proves that every accepted protocol state has been enumerated.

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

`JevActorHandler.t.sol` removes both the hand-written handler and `targetSenders()`. Its failure
requires a stable actor to deposit and later withdraw, plus a compatible getter hosted on a second
contract. It exercises scenario-level actor selection and cross-contract state reads rather than
only function selection.

With the recorded seed, Jev selected the complete same-actor lifecycle and found the failure in one
run and two calls using one request. Matched RNG completed 16 runs and 1,600 calls without finding
it. This isolates the automatically derived handler behavior; protocol-scale results are reported
separately and must include inference time.

## Matched hard-case fixtures

The same recorded seed and a 16-run, 100-depth budget produce the following current results. These
fixtures are intentionally checked in as both positive and negative controls; a model mode should
not be presented as a general improvement based only on its favorable cases.

| Fixture | Jev | RNG | Missing production when Jev misses |
| --- | ---: | ---: | --- |
| Auto-derived actor handler | found in 2 calls | missed in 1,600 calls | n/a |
| Pool ID omits a struct field ([#9782](https://github.com/foundry-rs/foundry/issues/9782)) | missed | missed | relational reuse across two structs |
| Rare modular predicate | missed | missed | arithmetic transforms of candidate values |
| Three magic state transitions | missed | found in 799 calls | concrete constants from branch predicates |
| Parade-style state growth | found in 27 calls | found in 21 calls | n/a |

The actor result validates scenario and identity selection; the misses define the next grammar
work. In particular, lifecycle selection alone cannot synthesize correlated structs or arithmetic
preimages, and Jev currently sees ABI structure rather than Solidity branch predicates.

Two protocol-scale harness checks used equal wall-clock budgets and the same seed. A 60-second Aave
v4 SCFuzzBench run found only its canary assertion in both modes; RNG covered more features. A
30-second Drips harness run found no invariant failure in either mode. Jev inference time is included
in those budgets. These are negative controls, not evidence of a protocol-scale improvement, and
show why further work should prioritize feedback-driven scenario diversity and richer value
productions before increasing the remote-choice rate.

The broader [Recon shortcut comparison](SCFUZZBENCH.md) runs the same native mode against all five
current pinned SCFuzzBench targets and records known-bug hits, coverage pulses, throughput, and the
current Superform function-limit fallback.

The generated-Solidity sample and native results exercise different fixtures and are not combined
performance evidence. No native maze result is claimed until the actual native binary has been
built and run under matched end-to-end budgets.
