// Stable wrapper that explicitly instantiates the wasm and wires it into the internal glue.
export * from './build/wasm_fmt_lint.internal.js'
import * as wbg from './build/wasm_fmt_lint.internal.js'

const url = new URL('./build/wasm_fmt_lint.wasm', import.meta.url)

// Create imports object from all __wbg_ and __wbindgen_ exports
const imports = {
  './wasm_fmt_lint.internal.js': Object.fromEntries(
    Object.entries(wbg)
      .filter(([key]) =>
        key.startsWith('__wbg_') || key.startsWith('__wbindgen_')
      )
      .map(([key, value]) => [key, value]),
  ),
}

const { instance } = await WebAssembly.instantiateStreaming(
  fetch(url),
  imports,
)
console.info('instance', instance)
wbg.__wbg_set_wasm(instance.exports)

// Initialize the externref table
if (wbg.__wbindgen_init_externref_table) {
  wbg.__wbindgen_init_externref_table()
}

// Call wbindgen start if it exists
if (instance.exports.__wbindgen_start) {
  ;(instance.exports.__wbindgen_start as () => void)()
}
