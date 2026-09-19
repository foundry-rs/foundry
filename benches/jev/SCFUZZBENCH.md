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

## One-seed smoke result

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

The dominant qualitative failure is repeated selection of one plausible scenario until the
32-request budget is exhausted. Superform also exceeds the current function cap. The next useful
experiment is therefore not a longer run of this exact policy: it is a diverse, paginated scenario
frontier followed by repeated, paired-seed SCFuzzBench trials.
