# Incremental JSON Output

Long-running command paths that accept `--json` emit newline-delimited JSON (NDJSON): one compact
JSON object per line on stdout as events occur. Human-readable progress and diagnostics remain on
stderr so stdout can be consumed line-by-line by tools.

Each line is a self-contained event object:

```json
{"schema_version":1,"event":"summary","data":{},"errors":[],"warnings":[]}
```

Top-level fields:

- `schema_version`: shared Foundry JSON schema version.
- `event`: stable machine-readable event discriminator.
- `data`: command-specific event payload.
- `errors`: structured errors associated with that event.
- `warnings`: structured warnings associated with that event.

Commands that finish quickly, and the legacy carve-outs listed below, may still emit a single
complete JSON object instead of an event stream. For event streams, overall command success is
reported by the final `summary` event payload. Intermediate events do not carry whole-command
success.

The default `forge test --json` wire format is an NDJSON event stream. This is a breaking change
from the previous aggregate object keyed by suite name.

## `forge build --json`

Events:

- `compile_start`: compilation started.
  - `paths`: explicit build paths, or an empty array when compiling the project.
- `compile_artifact`: one compiled artifact is available.
  - `source`: source path.
  - `name`: contract name.
  - `version`: compiler version.
- `summary`: terminal build result.
  - `success`: whether compilation completed without compiler errors.
  - `artifact_count`: number of compiled artifacts.
  - `error_count`: number of compiler errors.
  - `warning_count`: number of compiler warnings.
  - `output`: compiler output object, or `null` if the compiler did not produce output.
  - `error`: terminal error string, or `null` if no terminal command error occurred.

## `forge test --json`

Events:

- `test_result`: one test result is available.
  - `network`: optional per-test override network for multi-pass test runs.
  - `suite`: suite identifier.
  - `name`: test name.
  - `result`: serialized test result.
- `suite_summary`: one suite has finished.
  - `network`: optional per-test override network for multi-pass test runs.
  - `suite`: suite identifier.
  - `passed`: passed test count.
  - `failed`: failed test count.
  - `skipped`: skipped test count.
  - `total`: total test count.
  - `duration`: suite duration.
  - `warnings`: suite warnings.
- `summary`: terminal test result.
  - `success`: whether the run should be considered successful.
  - `suites`: suite count.
  - `passed`: passed test count.
  - `failed`: failed test count.
  - `skipped`: skipped test count.
  - `total`: total test count.
  - `wall_time`: elapsed wall-clock time.
  - `cpu_time`: accumulated test CPU time.

### Non-Streaming `forge test --json` Outputs

The NDJSON event stream applies to the default test execution path.

The following combinations emit non-streaming output:

- `forge test --gas-report --json`
- `forge test --summary --json`
- `forge test --list --json`

`--live-logs` is incompatible with `--json` because live logs are written directly to stdout.
`--junit` remains incompatible with `--json`.

## `forge coverage --json`

Events:

- `coverage_file`: coverage data for one source file.
  - `path`: source path.
  - `summary`: coverage counts for the file.
- `summary`: terminal coverage result.
  - `success`: whether the underlying test run should be considered successful.
  - `files`: number of files in the report.
  - `summary`: aggregate coverage counts.

Coverage summaries include `line_count`, `line_hits`, `statement_count`, `statement_hits`,
`branch_count`, `branch_hits`, `function_count`, and `function_hits`.
