# foundry-evm-sancov

SanitizerCoverage callbacks for coverage-guided fuzzing of native Rust code (precompiles, revm internals, etc.).

When forge is built with a `RUSTC_WRAPPER` that injects sancov flags for target crates, the fuzzer uses native code edge coverage to guide mutation. Comparison operands from instrumented code are also captured and injected into the fuzz dictionary.

## Config

```toml
[invariant]
sancov_edges = true
sancov_trace_cmp = true
corpus_dir = "corpus/invariant"
```

When `sancov_edges` is enabled, the EVM `EdgeCovInspector` is automatically disabled — sancov replaces EVM bytecode coverage as the guidance signal.
When `corpus_dir` is set and `sancov_edges` is disabled, EVM edge coverage and EVM comparison operands are collected automatically for coverage-guided fuzzing.
`sancov_trace_cmp` is independent and only adds comparison operands from sancov-instrumented native code.

## Replaying a corpus to AFL-`afl-showmap` files

To export coverage from a persisted corpus for cross-fuzzer comparison, use
`forge test --showmap-out <DIR>`. See [docs/dev/showmap.md](../../../docs/dev/showmap.md).

## Build

Create a `RUSTC_WRAPPER` that injects sancov flags for the crate(s) you want to instrument:

```bash
#!/usr/bin/env bash
RUSTC="$1"; shift
CRATE_NAME=""
PREV=""
for arg in "$@"; do
    [ "$PREV" = "--crate-name" ] && CRATE_NAME="$arg" && break
    PREV="$arg"
done

if [ "$CRATE_NAME" = "your_target_crate" ]; then
    exec "$RUSTC" "$@" \
        -Cpasses=sancov-module \
        -Cllvm-args=-sanitizer-coverage-level=3 \
        -Cllvm-args=-sanitizer-coverage-trace-pc-guard \
        -Cllvm-args=-sanitizer-coverage-trace-compares
else
    exec "$RUSTC" "$@"
fi
```

Then build:

```bash
RUSTC_WRAPPER=./sancov-wrapper.sh cargo build --profile fuzz --bin forge
```
