# wasm-fmt-lint

Browser-friendly WASM bindings for `forge-fmt` (format) and a stub for
`forge-lint`.

## Build

- Install Deno and add Rust target: `rustup target add wasm32-unknown-unknown`
- Build with wasmbuild (mirrors existing flow):

```
RUSTFLAGS='--cfg=getrandom_backend="wasm_js"' deno run -A jsr:@deno/wasmbuild@0.19.2 -p wasm-fmt-lint --out ./wasm-fmt-lint/dist
```

- Run the Vite playground:

```
(cd wasm-fmt-lint && deno task dev)
```

## API

- `fmt_default(source: string) -> { formatted } | { error }`
- `fmt_with_config(source: string, config: FormatterConfig) -> { formatted } | { error }`
- `fmt_config_default() -> FormatterConfig`
- `lint(source: string, options?: object)` → stubbed for now (returns empty
  diagnostics).

Notes:

- The lint API is stubbed to keep the surface stable while we upstream wasm
  support in `forge-lint` (rayon/threads, session emitter capture). Once ready,
  this will return structured diagnostics.
