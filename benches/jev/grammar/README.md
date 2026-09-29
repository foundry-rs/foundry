# Jev-guided sequence grammars

This opt-in prototype generates Solidity handlers for the checked-in local accounting model.
Jev's typed Choice distributions select grammar productions; Forge fuzzes the remaining values
and interleaves generated sequences with ordinary single-action handlers in a coverage-guided
invariant campaign. There are no inference requests in the execution loop.

It is a benchmark-side integration with existing Forge facilities, not a native engine change,
an arbitrary-contract adapter, or evidence of improved bug-finding performance. The model has
no external assets or network access; its correctness assertions are fixed, not model-generated.

## Grammar and execution

```text
sequence := action{1..8}
action   := deposit(amount) | withdraw(amount)
amount   := uint256 | balanceOf(address) | depositedBalanceOf(address)
address  := bytes20 | sender
sender   := 0x1 | 0x2 | 0x3
```

Each action binds its sender once. A balance expression is evaluated immediately before that
action, so later reads observe earlier state changes. For example, two consecutive
`deposit(balanceOf(sender))` actions use the remaining balance on the second call, not a cached
value. Sender impersonation happens after view calls, immediately before the ledger call.

Jev receives the local model description and the actual selected syntactic prefix, not runtime
balances it cannot know. One request per action contains four Choice questions. Their validated
probabilities are sampled independently with a seeded generator and, by default, a 20% uniform
exploration mixture. Unused address choices are discarded for raw integer amounts. Responses
cannot supply Solidity: only the fixed renderer consumes the validated typed AST.

Raw integers and address leaves remain Forge-generated parameters. Address leaves use full-width
`uint256` ABI carriers narrowed to `uint160` at the read site, preserving the bytes20 value domain
without noncanonical ABI-padding reverts during corpus mutation. Each handler is an atomic macro
action to Forge; assertions check deltas, expected reverts, and conservation after every inner
action. Forge can mutate the call sequence and leaves, but cannot shrink the grammar structure
inside a generated handler. Grammar shapes are fixed for a campaign, not adapted from feedback.

## Run

Requires Node.js 24 and Forge with invariant coverage-corpus support; validated with official
Forge v1.8.3 and Solidity 0.8.30. No npm dependencies or forge-std checkout are needed.

```sh
# Run all checks, including generated Solidity execution (otherwise that test is skipped).
FORGE_BIN=/absolute/path/to/forge node --test benches/jev/grammar/grammar.test.mjs

# Reproduce the real recorded decisions without credentials or network inference.
node benches/jev/grammar/generate.mjs \
  --replay benches/jev/grammar/recorded-decisions.json --output /tmp/jev-grammar-replay

# Run from the output directory: relative corpus/failure paths must stay project-local.
(cd /tmp/jev-grammar-replay && /absolute/path/to/forge test)

# Live mode bills six requests with the defaults below. Set OPENROUTER_API_KEY first.
node benches/jev/grammar/generate.mjs --live --output /tmp/jev-grammar-live \
  --seed 1 --sequences 2 --length 3 --exploration 0.2

# Uniform-production ablation; no model calls and the same grammar/renderer/oracles.
node benches/jev/grammar/generate.mjs --uniform --output /tmp/jev-grammar-uniform
```

Use Node's `--use-env-proxy` before the script path when required by your environment. Output
directories must not exist. Failed requests are not retried. Successful responses are saved before
validation and before the next paid request; incomplete recordings are rejected rather than
silently regenerated. Replay requires identical settings, grammar version, and request prefixes.
The maximum is eight sequences of eight actions (64 requests). Only the local model description,
grammar choices, and generated prefixes are sent, never private contract source or credentials.

Each output contains `decisions.json`, `programs.json`, generated Solidity, `foundry.toml`, and a
`manifest.json` of file hashes. The authorization header is not recorded. Inference and Forge run
as separate commands; Forge needs no model credentials. Start each independent comparison in a
fresh project directory and clear ambient `FOUNDRY_*`/`DAPP_*` overrides if present. Match Forge
seeds and total time budgets, including generation and compilation, when comparing strategies.

## Verified sample and limitations

The checked-in recording and [sample result](sample-result.json) contain six real decisions from
`typesafe/jev-1.13-20260917`, selecting
two three-action programs with seed 1. Requests took 2.426 seconds in total; the provider reported
$0.000190092 in usage cost. Credential-free replay produces identical generated file hashes.
The generated project passed 64 invariant runs / 1,024 handler calls, exercising both generated
sequences and both raw actions, and persisted coverage corpus entries. Twelve Node tests cover
validation, deterministic replay, late reads, sender binding, raw leaves, expected reverts, and
the mixed Forge campaign; the Forge integration must be enabled as shown above.

This sample demonstrates plumbing and semantics only. It has no injected defects and does not
measure additional findings. The earlier [arithmetic results](../RESULTS.md) concern a different
experiment and do not establish this grammar's efficacy. Representative held-out fixtures,
repeated model samples, uniform-grammar and hand-written baselines, and matched time/cost budgets
are still needed. Coverage also includes handler/oracle code, so corpus growth alone is not proof
of more useful target exploration.

## Design sources

The [Fuzzing Book's API Fuzzing chapter](https://www.fuzzingbook.org/html/APIFuzzer.html) motivates
separating grammar-based program generation from execution and maintaining dependencies between
generated calls. This implementation leaves concrete values to Forge and keeps state-dependent
expressions live, with an independently fixed correctness oracle.

[Verite, DOI 10.1145/3715720](https://blog.lazym.io/verite.pdf), particularly its action abstraction,
informed the treatment of semantically related arguments. This prototype does not implement its
profit-directed objective or reproduce its attacks or evaluation. The code is original; these
references describe the concepts used, not an equivalent implementation.
