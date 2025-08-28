import { dedent } from 'ts-dedent'
import * as W from './wasm_browser.ts'
console.info(W)
const inputElement = document.querySelector('textarea#input')
if (!inputElement) throw new Error('inputElement not found')
const outputElement = document.querySelector('pre#output')
const diagsElement = document.querySelector('pre#diagnostics')
const buttonElement = document.querySelector('button#btn-format')
const onInputElement = document.querySelector('input#format-on-input')

// Seed example source
inputElement.setAttribute(
  'value',
  dedent /* sol */`// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

contract Foo{event E(address indexed a, uint b);
  function bar(address a,uint256 b) external {
    emit E(a,b);
  }
}
`.trim(),
)

function runFormat() {
  if (!diagsElement) throw new Error('diagsElement not found')
  diagsElement.textContent = ''

  try {
    if (!inputElement || !outputElement) {
      throw new Error('inputElement or outputElement not found')
    }
    const cfg = W.fmt_config_default()
    const res = W.fmt_with_config(inputElement.value, cfg) as {
      formatted: string
    }
    outputElement.textContent = res.formatted ??
      JSON.stringify(res, null, 2)
  } catch (e) {
    const err = e as { error?: string } | string
    if (!diagsElement) throw new Error('diagsElement not found')
    diagsElement.textContent = typeof err === 'string'
      ? err
      : (err?.error ?? String(err))
  }
}

buttonElement?.addEventListener('click', (event) => {
  console.info(event)
  runFormat()
})
inputElement?.addEventListener('input', (_event) => {
  if (onInputElement?.checked) runFormat()
})

runFormat()
