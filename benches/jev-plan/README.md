# Experimental offline campaign plans

This experiment keeps the policy decision outside Forge. A recorded Choice response
allocates independent local campaign slices; the existing invariant engine owns all
transactions, arguments, assertions, shrinking, and corpus replay. Nothing changes
in normal RNG or stateless fuzzing. There is no provider client or new Foundry setting.

This is a plumbing prototype, **not evidence that Jev improves bug discovery**.
The included response is explicitly synthetic. It was not produced by Jev, and its
probabilities and confidence must not be interpreted as benchmark predictions.

## Boundary and candidates

The benchmark derives three candidates from the suite's existing invariant depth:
`short = depth / 2` (integer division), `base = depth`, and `long = depth * 2`.
These are campaign budgets, not ABI-derived transaction sequences. This intentionally
narrow experiment does not test the transaction grammar in
[Foundry #16936](https://github.com/foundry-rs/foundry/pull/16936).
It does not modify that PR or reproduce its model-specific correctness rules.

The candidate set is fixed before choosing a policy. Each seed produces five plans:

| Policy | Allocation |
| --- | --- |
| `baseline` | Always use the original suite depth; candidate-diversity ablation. |
| `uniform` | Uniform seeded sampling of the three candidates. |
| `round-robin` | Deterministic cycling through all candidates, with seed-rotated starting point. |
| `greedy` | Always use the recorded top choice, without exploration. |
| `weighted` | Sample the complete recorded distribution, mixed with uniform exploration. |

For `n` candidates, recorded probabilities `p`, confidence `c`, and exploration
parameter `e`, the local policy is `q[i] = (1 - (1-e)*c)/n + (1-e)*c*p[i]`.
Thus `q[i] >= e/n`, and zero confidence reduces to uniform. Confidence controls
trust in the distribution; it is not the probability that the policy is correct.
The floor is a probability floor, not a guarantee of visiting every candidate in a
finite run. The fixture's `e = 0.2` is an explicit experimental choice, not a tuned
or recommended default. Greedy intentionally has no floor to expose mode collapse.

The planner rejects incomplete/unknown choices, invalid distributions, nonfinite
numbers, inconsistent argmax answers, and mismatched locally derived depths.
Live recordings also require measured latency/cost and are bound to the requested
target repository and commit. The saved plan embeds the request, response,
provenance, weights, candidate set,
and every engine seed. Sampling draws and engine seeds use disjoint bytes of a versioned SHA-256 digest.
The runner reconstructs the plan before executing it and rejects edited schedules.

Each slice starts a fresh scfuzzbench workspace. There is no cross-slice corpus or
failure reuse: this measures allocation among independent restarts, not an adaptive
campaign that retains state. Equal wall budgets and engine seeds are shared across
policies. Exact plan replay does **not** imply identical execution under a wall-time
cutoff; CPU contention and the engine's coverage feedback can change the final corpus.

## Reproduce

Python 3's standard library is sufficient for planning. Execution uses the existing
[`foundry-scfuzzbench` workflow](../README.md#running-scfuzzbench-campaigns), including
its dependencies, target build, native invariant execution, showmap replay, and
analysis. Build that runner from the current checkout for new experiments.

```sh
cargo build --locked --profile profiling --bin foundry-scfuzzbench --bin forge
python3 -m unittest discover -s benches/jev-plan -v

python3 benches/jev-plan/plan.py compile \
  benches/jev-plan/synthetic-recording.json /tmp/jev-plan-1009.json \
  --seed 1009 --slices 1 --seconds 15 --exploration 0.2

python3 benches/jev-plan/plan.py run /tmp/jev-plan-1009.json /tmp/jev-plan-weighted \
  --policy weighted \
  --runner "$PWD/target/profiling/foundry-scfuzzbench" \
  --forge "$PWD/target/profiling/forge" \
  --target-repo https://github.com/scfuzzbench/drips-fuzzing-scfuzzbench.git \
  --target-ref 0ae8bd881e3c4f4a254f3cf92091e9997c5e88be \
  --scfuzzbench-ref 3bad5ea092113613c3d2fea03131a3a047acdaef
```

Use a new output directory for each policy and seed. Repeat with `baseline`,
`uniform`, `round-robin`, and `greedy`, seeds `1009`, `2003`, `3001`, and the
Origin Dollar repository at `299ec6b6bf5401b43efc3e9f7ee5cab3e76167c4`.
The run records exact runner argv and logs beside the plan. Each numbered slice
contains the existing runner's manifest, event/throughput/coverage tables, and
replay artifacts. Failure to collect required artifacts fails the run.

For an actual model experiment, capture a response to the **same request and
candidate set** with immutable model ID, full response/usage, measured latency,
cost, target revision, and pre-campaign context; mark provenance `live`.
No API request is made by this script. Do not relabel unrelated model recordings
or synthetic values as judgments about these candidates. The current minimal
request contains only candidate descriptions and depths; informative state summaries
and evidence that the decision has semantic content remain research work.

## Evidence and next gate

See [RESULTS.md](RESULTS.md) for the recorded screen and its limitations.
A model-quality evaluation requires relevant live recordings, longer independent
campaigns, more seeds, representative held-out suites, and catalog-mapped known bugs.
Compare uniform versus baseline to measure candidate diversity, then compare the
other schedulers versus uniform. Keep handlers and correctness oracles identical.
Include unique known bugs (deduplicating aliases/canaries), per-bug hit rates and
censored time-to-first, coverage unions, calls/throughput, and planning latency/cost.
Charge planning time against the same total budget when comparing end-to-end utility.
Do not promote this into core based on the plumbing screen.

## Source evidence

The [TypeSafe announcement](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
positions Jev as typed decisions inside ordinary software; its vendor-reported speed
claims are not Foundry measurements. The [Choice documentation](https://docs.typesafe.ai/primitives/choice)
returns a full distribution and confidence and recommends retaining alternatives.
The [Jev 1.13 jaggedness notes](https://docs.typesafe.ai/model-jaggedness/jev-1.13)
warn about numeric precision, indirection, irrelevant state, and adversarial content.
Those limitations motivate keeping candidate derivation, arithmetic, validation,
and correctness local. These sources were inspected on 2026-09-27.
