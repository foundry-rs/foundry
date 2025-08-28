// Stable browser wrapper around generated wasm + internal glue.
// Avoids calling non-existent __wbindgen_start by preferring __wasm_start when present.
import * as wasm from './build/wasm_fmt_lint.wasm'
export * from './build/wasm_fmt_lint.internal.js'
import { __wbg_set_wasm } from './build/wasm_fmt_lint.internal.js'

__wbg_set_wasm(wasm)

// Prefer wasm.__wbindgen_start if it exists; otherwise do nothing.
// Many modules don't require explicit start.
// deno-lint-ignore no-explicit-any
const anyWasm = wasm as any
if (typeof anyWasm.__wbindgen_start === 'function') {
  anyWasm.__wbindgen_start()
}
