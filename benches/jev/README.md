# Jev input-selection experiment

For typed, state-dependent Solidity sequence generation using Jev Choices alongside Forge's
coverage-guided invariant engine, see the separate [grammar prototype](grammar/README.md).

This opt-in experiment connects [TypeSafe Jev](https://docs.typesafe.ai/primitives/choice) through
OpenRouter's Decisions API to three checked-in synthetic arithmetic tests. Jev chooses an input
distribution from a fixed menu using only each test's specification. Forge generates the inputs,
evaluates the unchanged reference assertions, and shrinks failures. No generated Solidity or remote
response is executed as code.

This is a benchmark-side prototype, **not a native Forge fuzzer feature**. It neither discovers
targets nor operates on user-supplied contracts. The fixtures intentionally contain simple arithmetic
mutants; results establish plumbing and input-selection behavior, not production bug-finding efficacy.

## Run

Requires Node.js 24 and a Forge executable supporting JSON fuzz results. Solidity 0.8.30 is installed
by Forge if necessary. No npm packages or forge-std checkout are needed.

```sh
node --test benches/jev/experiment.test.mjs

# OPENROUTER_API_KEY must already be set. One request is billed by the provider.
# Only the checked-in arithmetic specifications and strategy descriptions are sent.
node benches/jev/experiment.mjs --live --forge /path/to/forge \
  --output /tmp/jev-first-run --seeds 20 --runs 256

# Replay the exact choices without another request or credentials.
node benches/jev/experiment.mjs --replay /tmp/jev-first-run/decisions.json \
  --forge /path/to/forge --output /tmp/jev-replay --seeds 20 --runs 256
```

Use Node's `--use-env-proxy` before the script path if your environment requires its configured proxy.
For a credential-free reproduction, pass `--replay benches/jev/recorded-decision.json`; this is the
saved live response used for the [initial results](RESULTS.md).
The output directory must not exist. Failed builds abort before inference, and failed inference is
not automatically retried or silently replaced with heuristic choices. The model is pinned to
`typesafe/jev-1.13`; the provider's resolved model and usage are retained in `decisions.json`.

## Comparison

Each seed runs the same three cases, with both mutant and clean implementations, in three modes:

| Mode         | Input selection                                                            |
| ------------ | -------------------------------------------------------------------------- |
| `broad`      | Unmodified Forge-generated `uint32` inputs                                 |
| `boundaries` | A deterministic mixture of near-zero, near-pivot, and near-maximum inputs  |
| `jev`        | Jev's selected distribution: broad, near-zero, near-pivot, or near-maximum |

Focused modes preserve the original input when `raw % 4 == 0`. This is nominally 25% broad
exploration; Forge's input distribution need not be uniform. All modes use the same binary, seeds,
thread count, per-test maximum fuzz runs, and fixed assertions. The fixture's `input` function applies
the selected distribution. No HTTP call occurs inside a fuzz execution. One decision is reused across
the paired seeds; these are not independent model samples.

Compilation is outside the measured trial. The command wall time includes process startup and
shrinking. Execution stops on a failure, so the run count is a **maximum**, not equal completed work.
`summary.json` includes the one-time inference duration in the Jev arm even during replay. Clean
controls are recorded separately and excluded from the mutant timing totals. This is a fixed-budget
detection comparison, not a matched wall-clock throughput comparison.

Each trial has an isolated failure-persistence directory; an earlier failure cannot contaminate a
later seed or mode through replay. Unexpected errors, zero tests, skipped tests, and clean-control
failures abort the experiment rather than increasing the finding count. Ambient `FOUNDRY_*`,
`DAPP_*`, and `JEV_*` settings are removed, and model credentials are excluded from Forge's child
environment.

## Inspect the evidence

`manifest.json` records the fixture hash, Forge version, model, seeds, and selected strategies.
`decisions.json` records the exact request/response and measured network duration; it never contains
the authorization header. `trials.json` records every test outcome, command duration, Forge's reported
fuzz-run count, and the first-failure run when available. Per-trial Forge JSON retains the synthetic
counterexample and test metadata. `summary.json` distinguishes repeated detections from distinct
synthetic defects. Preserve the entire output directory when sharing results.

These examples all have meaningful arithmetic pivots, so a cheap boundary heuristic is a strong
baseline. Equal findings or higher end-to-end latency must be reported as such: a working Jev
integration does not itself show that the model is useful. A broader claim needs representative,
held-out correctness fixtures, repeated model samples, and matched total time/cost budgets.

## Existing Foundry benchmarks

[`foundry-scfuzzbench`](../README.md#running-scfuzzbench-campaigns) owns invariant-campaign findings,
throughput, and corpus coverage reporting. [`foundry-bench`](../README.md#branch-vs-master-pr-body-workflow)
owns command timing. This small experiment does not replace either and has not established an
improvement on their workloads.
