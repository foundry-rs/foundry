import { dedent } from 'ts-dedent'
import * as W from './wasm_browser.ts'
const textAreaElement = document.querySelector('textarea#input')
if (!textAreaElement) throw new Error('textAreaElement not found')
const outputElement = document.querySelector('pre#output')
const diagnosticsElement = document.querySelector('pre#diagnostics')
const buttonElement = document.querySelector('button#btn-format')
const onInputElement = document.querySelector('input#format-on-input')

// Seed example source
textAreaElement.textContent = dedent /* sol */`// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

contract Foo{event E(address indexed a, uint b);
  function bar(address a,uint256 b) external {
    emit E(a,b);
  }
}
`.trim()

function runFormat() {
  if (!diagnosticsElement) throw new Error('diagnosticsElement not found')
  diagnosticsElement.textContent = ''

  try {
    if (!textAreaElement || !outputElement) {
      throw new Error('textAreaElement or outputElement not found')
    }

    const res = W.fmt_with_config(textAreaElement.value, W.fmt_config_default()) as {
      formatted: string
    }
    outputElement.textContent = res.formatted ??
      JSON.stringify(res, null, 2)
  } catch (e) {
    console.error(e)
    const err = e as { error?: string } | string
    if (!diagnosticsElement) throw new Error('diagnosticsElement not found')
    diagnosticsElement.textContent = typeof err === 'string'
      ? err
      : (err?.error ?? String(err))
  }
}

buttonElement?.addEventListener('click', (event) => {
  console.info(event)
  runFormat()
})
textAreaElement?.addEventListener('input', (_event) => {
  if (onInputElement?.checked) runFormat()
})

runFormat()
