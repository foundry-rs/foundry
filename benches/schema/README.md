# Benchmark result schema

`benchmark-result-v1.schema.json` defines a common benchmark result format.

Each document identifies the repository and commit being measured, optional pull request metadata, the runner, and one or more named benchmarks. Each benchmark has wall time and may include memory, custom counters, gas, solver, and compiler metrics.

Every metric records a numeric `value`, its `unit`, and the `statistic` represented by the value. Unknown or unmeasured fields are omitted rather than set to `null` or zero.

Recommended units are `second` for wall time, `byte` for memory and sizes, `gas` for gas, and `count` for counters. Metric names within each group should be stable snake_case identifiers.

`memory` represents peak resident memory for the benchmarked process tree.

Consumers should vendor the schema at a pinned Foundry commit so validation does not depend on network access. Incompatible changes require a new schema version.
