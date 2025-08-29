import vesper from '@shikijs/themes/vesper'

import { dedent } from 'ts-dedent'
import * as W from './wasm_browser.ts'
import * as monaco from 'monaco-editor-core'
import { shikiToMonaco } from '@shikijs/monaco'
import { createHighlighterCore } from 'shiki/core'
import { handleFormatError } from './format-error-handler.ts'
import { createOnigurumaEngine } from 'shiki/engine/oniguruma'

function debounce<T extends (...args: Array<any>) => any>(
  fn: T,
  delay: number,
) {
  let timeout: ReturnType<typeof setTimeout>
  return (...args: Parameters<T>) => {
    clearTimeout(timeout)
    timeout = setTimeout(() => fn(...args), delay)
  }
}

const highlighter = await createHighlighterCore({
  themes: [
    vesper,
    import('shiki/themes/github-light-default'),
  ],
  langs: [
    () => import('shiki/langs/solidity'),
  ],
  engine: createOnigurumaEngine(import('shiki/wasm')),
})

monaco.languages.register({ id: 'solidity' })

const resultElement = document.querySelector('div#result')
const formatButtonElement = document.querySelector('button#format')
const diagnosticsElement = document.querySelector(
  'div#diagnostics',
) as HTMLDivElement
const onInputElement = document.querySelector('input#format-on-input')

// const getDefaultConfiguration = () => W.fmt_config_default() ?? ({})

// Seed example source
const exampleSource = dedent /* sol */`// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;
contract Foo{event E(address indexed a, uint b);
  function bar(address a,uint256 b) external { emit E(a,b); }
}`.trim()

const runFormatDebounced = debounce(runFormat, 1_500)

function runFormat(params: {
  source: string
}) {
  if (!diagnosticsElement) throw new Error('diagnosticsElement not found')
  diagnosticsElement.innerHTML = ''

  try {
    const { formatted: output } = W.fmt_with_config(
      params.source,
      W.fmt_config_default(),
    ) as { formatted: string }

    if (!resultElement) throw new Error('resultElement not found')

    const html = highlighter.codeToHtml(output, {
      theme: 'vesper',
      lang: 'solidity',
    })
    resultElement.innerHTML = html

    editor.setValue(output)
  } catch (error) {
    console.error(error)
    if (!diagnosticsElement) throw new Error('diagnosticsElement not found')

    // Use the error handler with ANSI colors preserved by default
    const errorResult = handleFormatError(error)
    diagnosticsElement.innerHTML = errorResult.ansiHtml
  }
}

formatButtonElement?.addEventListener(
  'click',
  (_) => runFormat({ source: editor.getValue() }),
)

shikiToMonaco(highlighter, monaco)

const editorElement = document.querySelector('div#editor')
if (!editorElement) throw new Error('editorElement not found')

const editor = monaco.editor.create(editorElement, {
  value: exampleSource,
  language: 'solidity',
  theme: 'vesper',
  fontFamily: 'Fira Code',
  fontSize: 15,
  fontLigatures: true,
  minimap: {
    enabled: false,
  },
  scrollBeyondLastLine: false,
  scrollbar: {
    alwaysConsumeMouseWheel: false,
  },
  renderLineHighlight: 'none',
  padding: {
    top: 10,
  },
  guides: {
    indentation: false,
  },
  folding: false,
  lineNumbersMinChars: 3,
})

const splitPanel = document.querySelector('sl-split-panel')

globalThis.addEventListener('resize', (_event) => {
  if (globalThis.innerWidth < 900) splitPanel?.setAttribute('vertical', '')
  else splitPanel?.removeAttribute('vertical')

  editor.layout({
    width: editorElement.offsetWidth,
    height: editorElement.offsetHeight,
  })
})

globalThis.addEventListener('sl-reposition', (_event) => {
  editor.layout({
    width: editorElement.offsetWidth,
    height: editorElement.offsetHeight,
  })
})

editor.onDidChangeModelContent((_event) => {
  if (onInputElement?.checked) {
    runFormatDebounced({ source: editor.getValue() })
  }
})

runFormat({ source: exampleSource })
