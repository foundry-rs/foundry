import * as wasm from './pkg/wasm_fmt_lint.js'

self.onmessage = (event) => {
  const { type, bytes } = event.data

  console.info(`New event: ${type}`)

  if (event.data.type === 'INIT_WASM') {
    wasm.initSync({ module: bytes })
    self.postMessage({ type: 'WASM_FETCHED' })
  }

  if (event.data.type === 'FORMAT') {
    const { formatted: output } = wasm.fmt_with_config(
      event.data.input,
      wasm.fmt_config_default(),
    )
    self.postMessage({ type: 'FORMAT_DONE', output })
  }
}

self.postMessage({ type: 'FETCH_WASM' })
