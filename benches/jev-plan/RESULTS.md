# Plumbing screen: 2026-09-27

**No live Jev calls were made. No Jev bug-discovery or performance improvement is
established.** The fixture is handwritten and labeled `synthetic`, because no Jev
credentials or relevant campaign-policy recording was available. The real recording
inspected in #16936 concerns a different action grammar; it was not relabeled or
reused as a prediction about these campaign budgets.

The screen runs real stateful suites through `foundry-scfuzzbench`: Drips at
`0ae8bd881e3c4f4a254f3cf92091e9997c5e88be` and Origin Dollar at
`299ec6b6bf5401b43efc3e9f7ee5cab3e76167c4`. Both configure depth 100, yielding the
same locally derived candidates 50/100/200. Five policies use seeds 1009/2003/3001,
one independent 15-second slice per trial, and one local invariant worker.
There are 30 measured trials. Three seed streams run concurrently on the same Mac;
the policy order is rotated by stream. CPU contention and unequal setup duration
make this a plumbing screen, not an isolated throughput benchmark.

All arms use the same existing profiling Forge binary at reported commit
`d55e99f992dd30d75f3e8bc886db1aa96ada675f`, **not a fresh build of this PR or current
master**. Binary hashes and environment details are in [evidence.json](evidence.json).
The runner pins `tempoxyz/scfuzzbench` at
`3bad5ea092113613c3d2fea03131a3a047acdaef`. Each trial receives a fresh cloned target,
corpus and failure directory; compilation and post-campaign showmap replay are
outside the 15-second campaign budget. There were no `FOUNDRY_*` overrides.

## Results

| Suite | Policy | Unique known bugs | Known bug × seed hits | Median calls | Calls/budget s | Median showmap locations |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| drips | baseline | N/A | N/A | 20,400 | 1,360 | 39,227 |
| drips | uniform | N/A | N/A | 27,800 | 1,853 | 39,296 |
| drips | round-robin | N/A | N/A | 14,800 | 987 | 39,238 |
| drips | greedy | N/A | N/A | 14,000 | 933 | 39,185 |
| drips | weighted | N/A | N/A | 25,500 | 1,700 | 39,337 |
| origin-dollar | baseline | 4/12 | 12/36 | 372,500 | 24,833 | 14,547 |
| origin-dollar | uniform | 5/12 | 13/36 | 308,350 | 20,557 | 14,522 |
| origin-dollar | round-robin | 5/12 | 14/36 | 310,000 | 20,667 | 14,547 |
| origin-dollar | greedy | 6/12 | 14/36 | 374,800 | 24,987 | 14,547 |
| origin-dollar | weighted | 4/12 | 12/36 | 363,800 | 24,253 | 14,547 |

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
| total-supply-below-balances | 0/3 | 1/3 | 2/3 | 1/3 | 0/3 |
| transfer-sender-shortfall | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| transfer-within-balance-reverts | 0/3 | 0/3 | 0/3 | 1/3 | 0/3 |

The other six catalog entries have 0/3 hits for every policy. Every Origin Dollar
trial hits at least one known bug. Median time-to-first is in the starting clock
second for all policies; the weighted arm has a 0–1 second observed range.

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
