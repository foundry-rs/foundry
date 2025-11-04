# Remote tracing in Forge

Remote tracing lets Forge delegate trace execution to a live Cosmos-EVM node via JSON-RPC instead of interpreting locally with REVM. This is useful when you want to reproduce the chain’s execution behavior and precompiles exactly as your node implements them.

## What it does

- Uses the node’s `debug_traceCall` to execute and trace a call on the selected fork block (latest by default)
- Prints a compact nested call tree derived from geth’s `callTracer` result
- Keeps the rest of the Forge UX intact (filtering, verbosity, junit, etc.)

## Requirements

- A Cosmos-EVM compatible RPC that supports the `debug_` namespace (e.g. Evmos, Injective)
- A `--fork-url` so Forge knows which endpoint to call

## CLI usage

```sh
forge test \
  --fork-url https://rpc.evmos.dev \
  --trace-source remote \
  --trace
```

You can also set it in `foundry.toml`:

```toml
[profile.default]
trace_source = "remote"
```

Notes:
- If `--trace-source` is not provided, the default is `local` (REVM-based tracing)
- Remote tracing currently fetches only the test Execution trace (not Setup/Deployment)

## Output

When remote tracing is active and traces are shown, Forge prints a simple call tree similar to:

```
CALL 0x...from -> 0x...to gasUsed=24567
  CALL 0x... -> 0x... gasUsed=3245
    CALL 0x... -> 0x... gasUsed=410
```

This comes directly from the node’s `callTracer`. Labels and ABI decoding are not yet applied to these remote frames.

## Limitations (current)

- Uses built-in `callTracer` only; other tracers like `prestateTracer` are not exposed via CLI yet
- Remote Execution frames are printed with a minimal renderer (no ABI decoding / labels)
- Requires the RPC to enable and serve `debug_` methods

## Troubleshooting

- “method not found” or 403: the node likely does not expose `debug_traceCall`
- Empty/partial traces: some nodes restrict fields or have implementation differences; try a different endpoint
- No traces printed: ensure `--trace` is set and verbosity is sufficient for your test outcome

## Example

```sh
forge test -vvvv \
  --fork-url https://evmos-evm.publicnode.com \
  --trace-source remote \
  --match-test testTransfer
```

If the test fails or `-vvvv` is used, you’ll see a remote call tree for the Execution phase.


