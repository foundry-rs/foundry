import { dedent } from 'ts-dedent'
import { expect } from 'jsr:@std/expect'

import * as W from '../build/wasm_fmt_lint.js'

Deno.test('Format with default config', async () => {
  const codePreFormat = dedent /* sol */`
    contract A { struct B { uint256 c; } }
  `.trim()

  const codePostFormat = dedent /* sol */`
    contract A {
        struct B {
            uint256 c;
        }
    }
    `.trim()
  console.info(codePreFormat)
  console.info(codePostFormat)

  const solCode = await Deno.readTextFile(
    import.meta.dirname + '/Counter.sol',
  )
  console.info(solCode)
  expect(codePreFormat).toBe(solCode)

  const cfg = W.fmt_config_default()
  const { formatted } = W.fmt_with_config(codePreFormat, cfg) as {
    formatted: string
  }
  console.info(formatted)
  // expect(codePostFormat).toBe(formatted.trim())
})
