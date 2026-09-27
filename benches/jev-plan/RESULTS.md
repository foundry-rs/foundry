# Plumbing screen: 2026-09-27

**The 30-trial screen below made no live Jev calls and establishes no Jev
bug-discovery or performance improvement.** Its fixture is handwritten and labeled
`synthetic`. A later live follow-up is recorded separately below.

## Live follow-up

Two Jev 1.13 Choice calls used only the public Origin Dollar harness and pinned
revision. The first chose the suite's existing depth 100 with 18% confidence. After
the planner's exploration floor, its weighted schedule is identical to uniform in
35 of 36 deterministically compiled slices across three seeds. The second chose the complete
existing campaign profile with 40% confidence over restart-heavy, deep, and
mutation-heavy alternatives. That selected arm is exactly the baseline, so rerunning
it cannot measure a model effect.

Each call cost $0.001 and took 5.096 and 4.889 seconds. The full public requests,
resolved model ID, response probabilities, confidence, and token usage are in
`live-origin-dollar-recording.json` and
`live-origin-dollar-profile-recording.json`. Payment credentials and wallet data are
not retained. This is negative evidence for this narrow planning use case: the live
model supplied no distinct configuration to test, and the draft remains experimental.

The screen runs real stateful suites through `foundry-scfuzzbench`: Drips at
`0ae8bd881e3c4f4a254f3cf92091e9997c5e88be` and Origin Dollar at
`299ec6b6bf5401b43efc3e9f7ee5cab3e76167c4`. Both configure depth 100, yielding the
same locally derived candidates 50/100/200. Five policies use seeds 1009/2003/3001,
one independent 15-second slice per trial, and one local invariant worker.
There are 30 measured trials. Three seed streams run concurrently on the same Mac;
the policy order is rotated by stream. CPU contention and unequal setup duration
make this a plumbing screen, not an isolated throughput benchmark.

All arms use fresh profiling Forge and `foundry-scfuzzbench` binaries built from
the initial PR commit `d295a23ec328a8e0edd7dfda1625e65b79fe2687`. Binary hashes and
environment details are in [evidence.json](evidence.json).
The runner pins `tempoxyz/scfuzzbench` at
`3bad5ea092113613c3d2fea03131a3a047acdaef`. Each trial receives a fresh cloned target,
corpus and failure directory; compilation and post-campaign showmap replay are
outside the 15-second campaign budget. There were no `FOUNDRY_*` overrides.

## Results

| Suite | Policy | Unique known bugs | Known bug × seed hits | Median calls | Calls/budget s | Median showmap locations |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| drips | baseline | N/A | N/A | 19,000 | 1,267 | 39,233 |
| drips | uniform | N/A | N/A | 17,200 | 1,147 | 39,242 |
| drips | round-robin | N/A | N/A | 6,200 | 413 | 39,214 |
| drips | greedy | N/A | N/A | 4,600 | 307 | 39,192 |
| drips | weighted | N/A | N/A | 18,200 | 1,213 | 39,270 |
| origin-dollar | baseline | 4/12 | 12/36 | 272,900 | 18,193 | 14,550 |
| origin-dollar | uniform | 4/12 | 12/36 | 265,900 | 17,727 | 14,512 |
| origin-dollar | round-robin | 5/12 | 13/36 | 292,900 | 19,527 | 14,547 |
| origin-dollar | greedy | 5/12 | 13/36 | 273,800 | 18,253 | 14,547 |
| origin-dollar | weighted | 4/12 | 12/36 | 251,300 | 16,753 | 14,512 |

`calls/s` is final calls divided by the 15-second budget, not a sampled steady-state
rate. Coverage is the median number of distinct showmap instruction locations in
the replayed corpus, including the harness; it is not branch coverage or a percentage.
The runner's progress-rate CSVs are empty with this Forge binary. They are not treated
as zero. Linux `/proc` resource probes also do not work on this macOS host.

The Origin Dollar known-bug catalog contains 12 entries. Matching uses the public
SCFuzzBench catalog at `de70065957a32fc5dd5fba3e53bb60f8564c722c`, checks the target
revision, resolves event selectors through the compiled `CryticTester` method
identifiers, and uses that revision's `analysis/known_bug_report.py` to deduplicate
aliases and exclude canaries. Drips' catalog entry is pinned to a different revision
(`c50d160ead6bf82b1d1071e853dc74da0b23f595`), so its unique-known-bug and hit-rate
metrics are unavailable, not zero. Raw assertion events are retained in
[samples.csv](samples.csv), but are not called unique bugs.

Origin Dollar per-bug hit rates (three trials per policy):

| Catalog ID | Baseline | Uniform | Round-robin | Greedy | Weighted |
| --- | ---: | ---: | ---: | ---: | ---: |
| burn-balance-shortfall | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| change-supply-mismatch | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| rebasing-credits-per-token-increase | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| total-supply-below-balances | 0/3 | 0/3 | 1/3 | 1/3 | 0/3 |
| transfer-sender-shortfall | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |

The other seven catalog entries have 0/3 hits for every policy. Every Origin Dollar
trial hits at least one known bug, in the starting clock second.

Time-to-first uses the runner log's campaign-start timestamp and the first matching
failure event's epoch timestamp, both at whole-second precision. The existing
`events.csv` elapsed column starts at the first JSON event and is unsuitable here.
Values of zero mean the event occurred in the starting clock second, not zero work.
Missing bug hits are right-censored at 15 seconds. There is no inference time to
include: actual model requests = 0, actual model spend = $0, model latency = N/A.
An eventual live comparison must charge planning latency against its total budget.

## Replay and interpretation

[samples.csv](samples.csv) records each seed, selected depth, engine seed, plan hash,
calls, showmap location count, known-bug count, and time-to-first. Recompile the checked-in
fixture with the README command for each seed to reconstruct the exact plan; its
SHA-256 is over the emitted JSON bytes. Every raw run also records exact runner argv,
the full embedded fixture, runner manifest, logs, and replay corpus in its local output.
The committed evidence contains aggregated outcomes, not generated failure sequences.

The 1009 uniform and weighted schedules both select depth 50; the 2003 schedules both
select 100; only 3001 differs (uniform 50, weighted 100). Any differences between the
identical schedules expose wall-budget and execution noise. With one slice, the
round-robin arm exercises only its seed-selected starting candidate; multi-slice
cycling and exploration are covered by the planner tests, not this campaign screen.

This validates the architecture plumbing and the equality of candidate sets and
correctness oracles. It does not resolve candidate quality, model usefulness,
adaptive corpus retention, long-run exploration, or statistical significance.
The unchanged-depth baseline and uniform arm separate candidate diversity from
scheduler effects in the design, but these short, noisy trials cannot establish a
reliable effect size. No change to core is justified by these results.

## Frontier-selection follow-up

A separate screen captured 256 stateful Origin Dollar comparison frontiers from one
20-second, depth-100 campaign. Five arms started from exact copies of that artifact,
received 16 one-second symbolic attempts, concretely replayed every candidate, and
then ran only one depth-one invariant iteration. The first Jev tournament compared
all frontiers from concrete metadata. The second retained 92 one-sided frontiers
with nontrivial Solidity source-map fragments before ranking them. Each candidate in
each tournament appeared in four randomized groups.

| Selector | Replayed entries | Showmap locations | Property failures | Solve wall |
| --- | ---: | ---: | ---: | ---: |
| Foundry automatic | 1 | 1,160 | 0 | 4.30s |
| Jev metadata | 2 | 7,355 | 0 | 5.45s |
| Random, all frontiers | 3 | 7,341 | 0 | 4.22s |
| Jev source-aware | 3 | 7,693 | 0 | 3.75s |
| Random, source-filtered | 2 | 2,667 | 0 | 4.11s |

The source-aware arm crossed more replayable branches than the automatic selector,
but did not beat unrestricted random selection on accepted entries and produced no
bug. Showmap totals include each retained concrete prefix, so a deeper sequence can
inflate the total without representing equivalent incremental coverage. They are not
evidence of a bug-discovery gain.

The two tournaments made 176 Jev 1.13 calls costing $0.176. Their aggregate reported
latency was 703.751 seconds; median calls took about 3.8 seconds. Calls were issued
four at a time for the experiment, but an in-engine scheduler still cannot treat
that latency or external dependency as free. The bounded result is therefore
negative: source context makes the judgments less arbitrary, but neither tested Jev
policy establishes enough value to replace Foundry's local selector. The exact
configuration and aggregate outcomes are in
[frontier-selection-evidence.json](frontier-selection-evidence.json).
