// Stable browser wrapper around generated wasm + internal glue.
// Loads wasm via `new URL(..., import.meta.url)` so bundlers rewrite the asset URL.
export * from './build/wasm_fmt_lint.internal.js'
import { __wbg_set_wasm } from './build/wasm_fmt_lint.internal.js'

const wasmUrl = new URL('./build/wasm_fmt_lint.wasm', import.meta.url)
const { instance } = await WebAssembly.instantiateStreaming(fetch(wasmUrl), {})
__wbg_set_wasm(instance.exports as any)
