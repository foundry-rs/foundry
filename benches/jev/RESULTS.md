# Initial synthetic-fixture result

Jev successfully selected input distributions consumed by Forge. It improved detection reliability
against broad sampling in these fixtures, but **did not find more distinct defects than either
baseline, and did not improve total wall time over the boundary heuristic**.

The comparison used Forge v1.8.3 at `cae51ad458f6abb64852b7709eb784352429825d` (the official `dist`
release), Solidity 0.8.30, one worker, seeds 1 through 20, and a maximum of 256 fuzz inputs per test.
All arms used the same executable: this is a comparison of fixture input strategies, not a change
to Forge's native engine or a profiling-build branch-versus-master comparison.

| Metric                                           |   Broad | Boundary heuristic |     Jev |
| ------------------------------------------------ | ------: | -----------------: | ------: |
| Detected mutant trials                           |  7 / 60 |            60 / 60 | 60 / 60 |
| Distinct synthetic defects detected              |   3 / 3 |              3 / 3 |   3 / 3 |
| Clean control failures                           |  0 / 60 |             0 / 60 |  0 / 60 |
| Median first-failure input among detected trials |     153 |                 32 |      11 |
| Sum of mutant command wall times                 | 1.445 s |            1.402 s | 1.395 s |
| Including one live model request                 | 1.445 s |            1.402 s | 1.979 s |

There were 360 total trials, including clean controls. Misses reached the input limit, so the broad
arm's conditional median excludes 53 censored trials and is not directly comparable with the others.
The 8 ms difference in execution-only time between the boundary and Jev arms is too small to support
a speed claim. Timings exclude compilation and control trials but include process startup and
shrinking. There is no matched wall-clock-budget experiment or inference-latency distribution here.

One live request to OpenRouter resolved `typesafe/jev-1.13` to `typesafe/jev-1.13-20260917`, took
584.308 ms, and selected `pivot` for each specification. The response reported 753 input tokens,
133 output tokens, and a cost of USD 0.000031626. The same response was replayed across the paired
20-seed experiment, with its original latency charged once to the Jev timing total. Seeds vary
Forge's input generation, not the model judgment. The model saw only specifications and the fixed
strategy menu, not mutant code, assertions, or observed failures.

The fixture SHA-256 is `eff9b6613da79f53a70a95b8568ed007ea50af8ea80a8fe6c6bf0dfcd8c359fa`.
To rerun with the saved live decision and your own measurements:

```sh
node benches/jev/experiment.mjs --replay benches/jev/recorded-decision.json \
  --forge /path/to/forge-v1.8.3 --output /tmp/jev-reproduction --seeds 20 --runs 256
```

These three examples were deliberately constructed around arithmetic pivots. They are not a
held-out benchmark and cannot establish generalization to other code. The current evidence supports
the integration and faster detection in input-count terms on this fixture set; it does not establish
additional real-world findings or a benefit over inexpensive heuristic guidance at equal total time.
