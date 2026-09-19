# Recon shortcut comparison

This is a preliminary comparison against the handlers produced by Recon's
[coverage phase 2](https://github.com/Recon-Fuzz/recon-magic-framework/blob/HEAD/prompts/agent/coverage-phase-2.md).
That workflow writes shortcut functions which call prerequisites, read exact live state, and then
call a target function. The comparison uses the immutable targets in
[SCFuzzBench's manifest](https://github.com/scfuzzbench/scfuzzbench/blob/main/benchmarks/targets.json),
including the three suites adapted from Recon campaigns.

## Method

Both arms use this branch's Forge binary against the same target checkout and existing Recon-style
handler. The only changed input is `--invariant-tx-generator rng` versus `jev`. Each campaign uses
one worker, the same seed, a 30-second fuzzing timeout, depth 100, and a fresh corpus. Compilation is
excluded. Model latency is included. The latest progress pulse before the timeout supplies edge,
feature, and transaction counts; post-timeout shrinking is excluded. Ground-truth bug hits use
[SCFuzzBench's revision-locked catalog](https://github.com/scfuzzbench/scfuzzbench/blob/main/benchmarks/known_bugs.json).

```sh
FOUNDRY_INVARIANT_TIMEOUT=30 FOUNDRY_INVARIANT_RUNS=500000000 \
FOUNDRY_INVARIANT_DEPTH=100 forge test --mc CryticToFoundry \
  --match-test 'invariant_' --invariant-workers 1 \
  --invariant-tx-generator jev \
  --fuzz-seed 0x585f37fbac9620027325a193e979e3c93c87b069b15dce6c903f5f460436f916
```

## Initial one-seed smoke result

This table predates the paginated scheduler below and records the bottlenecks which motivated it.

| Target | RNG findings | Jev findings | RNG pulse | Jev pulse | Result |
| --- | --- | --- | ---: | ---: | --- |
| [Aave v4](https://github.com/scfuzzbench/aave-v4-scfuzzbench/tree/459b020058f4a65d18eb1481083b75c766340124) | invariant canary | both canaries | 118 edges / 15 features / 1,500 txs | 111 / 22 / 800 | Jev finds the assertion canary but no real catalog bug exists |
| [Superform v2](https://github.com/scfuzzbench/superform-v2-periphery-scfuzzbench/tree/102eeaf5e2f49963bdd007c7358739b0a0e59e84) | 2/27 invariants | fallback: 2/27 plus assertion canary | 405 / 13 / 1,500 | 396 / 8 / 2,100 | invalid comparison: 128-function guard disables Jev |
| [Liquity governance](https://github.com/scfuzzbench/liquity-V2-gov-scfuzzbench/tree/5610870e7e91fe6e29f8cfb0c0ae316fe9784e10) | 4/8 invariants plus assertion canary | same | 485 / 183 / 6,300 | 379 / 142 / 4,200 | equal findings, lower Jev coverage and throughput |
| [Origin Dollar](https://github.com/scfuzzbench/origin-dollar-scfuzzbench/tree/299ec6b6bf5401b43efc3e9f7ee5cab3e76167c4) | 4/12 catalog bugs; 12 assertion events | same | 189 / 3 / 54,400 | 206 / 5 / 35,200 | equal known bugs; slightly more coverage, much lower throughput |
| [Drips](https://github.com/scfuzzbench/drips-fuzzing-scfuzzbench/tree/c50d160ead6bf82b1d1071e853dc74da0b23f595) | 1/1 catalog bug through both aliases | 1/1 through one alias | 114 / 51 / 3,900 | 117 / 75 / 2,800 | equal known bug; more Jev coverage, lower throughput |

These runs do not establish a statistically meaningful improvement. They do show that native Jev
can run on four of the five current SCFuzzBench targets and preserve the known-bug hit set in the
two targets with cataloged real bugs. In this sample it improves feature discovery on Aave, Origin
Dollar, and Drips, while regressing Liquity and paying substantial inference overhead.

The dominant qualitative failure was repeated selection of one plausible scenario until the
32-request budget was exhausted. Superform also exceeded the original function cap.

## Paginated scheduler

The follow-up implementation removes the 128-function guard, bounds remote criteria rather than
the local grammar, derives lifecycle pairs without the full quadratic cross-product, interleaves
direct and scenario productions, rotates through the entire candidate set, withholds recent model
choices, and returns up to eight selected actions or scenarios per request. This makes Superform's
536 productions and 1,560 scenarios usable and amortizes inference over a larger local batch.

On Superform, one same-seed 30-second smoke run now executes the Jev path rather than RNG fallback.
It finds the same 2/27 invariants as RNG and reaches 430 edges / 10 features / 200 transactions.
This validates large-ABI support but is not a performance win.

The Aave comparison uses four fresh, paired corpora. Three pairs use seeds 1, 2, and 3; the fourth
uses the command's recorded seed above. The latest pulse is reported for each 30-second campaign.

| Seed | RNG edges / features / txs | Jev edges / features / txs |
| --- | ---: | ---: |
| recorded | 118 / 15 / 1,500 | 124 / 46 / 1,700 |
| 1 | 156 / 34 / 2,000 | 148 / 27 / 1,700 |
| 2 | 143 / 29 / 1,500 | 178 / 47 / 1,600 |
| 3 | 147 / 27 / 1,600 | 158 / 40 / 1,600 |
| median | **145 / 28 / 1,550** | **153 / 43 / 1,650** |

The median delta is +5.5% edges, +53.6% features, and +6.5% completed transactions. Both modes find
the invariant canary in every run. Jev additionally reaches the assertion canary in 2/4 runs versus
0/4 for RNG. One seed regresses, so this is promising short-budget evidence rather than a claim of
statistical significance or additional real bug discovery.

For scale, Foundry's published
[SCFuzzBench tracking issue](https://github.com/foundry-rs/foundry/issues/14437) reports the Aave v4
24-hour baseline as Echidna 10 [9,10], Medusa 10 [9,10], and Foundry 3 [3,3] broken invariants.
Those historical 24-hour numbers are not directly comparable to these 30-second branch trials, but
they identify the long-run gap this scheduler is intended to attack. A full SCFuzzBench campaign
with repeated 24-hour instances remains required before claiming parity with the other engines.

## ABI-derived protocol lifecycles

The next scheduler revision derives bounded protocol lifecycles from common ABI verbs. For
contracts exposing actor or asset selection, collateral, supply/deposit, borrow, oracle/config,
and exit/liquidation operations, it offers Jev complete candidate sequences in both ABI-random
and dictionary-backed forms. Jev still chooses the lifecycle; local generation supplies most
arguments and conventional fuzz actors (`0x10000`, `0x20000`, and `0x30000`) preserve compatibility
with pre-funded harness actors. Jev mode also treats a single ABI `bool` return of `false` as an
invariant failure, matching Echidna-style property harnesses; RNG mode retains Foundry's existing
return-value behavior.

One fresh, matched Aave v4 smoke pair used seed `0x4`, one worker, depth 100, a 180-second timeout,
and separate empty corpora. Model latency is included. Failure events emitted during generation
are counted once by invariant or assertion selector, with the duplicate canary signal collapsed.

| Mode | Distinct bugs | Non-canary bugs | Calls at final pulse | Findings |
| --- | ---: | ---: | ---: | --- |
| RNG | 1 | 0 | 12,400 | canary |
| Jev | **4** | **3** | 8,300 | canary; `totalBorrowedLessThanSupplied_v0`; `mintFeeShares`; `shouldNotBecomeLiquidatable` |

Jev reached `totalBorrowedLessThanSupplied_v0` after 32 seconds, `mintFeeShares` after 105 seconds,
and `shouldNotBecomeLiquidatable` after 163 seconds. This single three-minute campaign exceeds the
published Foundry Aave v4 24-hour median of three broken invariants, but it is not yet evidence of
Echidna/Medusa parity: both report a median of ten over 24 hours. Repeated release-pinned trials are
the next gate.
