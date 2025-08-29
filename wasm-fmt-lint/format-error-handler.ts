/**
 * Comprehensive error handler for forge-fmt errors with ANSI codes
 */

// Regex to strip all ANSI escape sequences
const ANSI_REGEX =
  /[\u001b\u009b][[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]/g

export interface ParseError {
  type: string
  line: number
  column: number
  endLine?: number
  endColumn?: number
  file: string
  message: string
  snippet?: string
  expected?: string[]
  found?: string
}

export function stripAnsi(str: string) {
  return str.replace(ANSI_REGEX, '')
}

export function parseFormatError(errorStr: string): ParseError {
  const clean = stripAnsi(errorStr)

  // Parse error type (e.g., "Error: ParserError")
  const typeMatch = clean.match(/Error:\s*(\w+)/)
  const errorType = typeMatch?.[1] || 'ParseError'

  // Parse location (e.g., "[ :2:18 ]")
  const locationMatch = clean.match(/\[\s*([^:]*):(\d+):(\d+)\s*\]/)
  const file = locationMatch?.[1]?.trim() || 'input.sol'
  const line = locationMatch ? parseInt(locationMatch[2], 10) : 1
  const column = locationMatch ? parseInt(locationMatch[3], 10) : 1

  // Parse the error message (e.g., "unrecognised token 'LARGE_NUM', expected ...")
  const messageMatch = clean.match(
    /unrecognised token '([^']+)'.*expected (.+?)(?:\n|$)/m,
  )
  const found = messageMatch?.[1]
  const expectedStr = messageMatch?.[2]
  const expected = expectedStr
    ? expectedStr.split(',').map((s) => s.trim().replace(/['"]/g, ''))
    : undefined

  // Extract code snippet if present
  const snippetMatch = clean.match(/\d+\s*│\s*(.+?)(?:\n|$)/)
  const snippet = snippetMatch?.[1]?.trim()

  // Build error message
  let message = `Syntax error`
  if (found) {
    message += `: unexpected token '${found}'`
    if (expected && expected.length > 0) {
      message += `, expected ${expected.map((e) => `'${e}'`).join(' or ')}`
    }
  }

  return {
    type: errorType,
    line,
    column,
    file,
    message,
    snippet,
    expected,
    found,
  }
}

export function formatErrorHtml(error: ParseError) {
  const parts = [
    `<div class="format-error">`,
    `  <div class="error-header">`,
    `    <span class="error-type">${error.type}</span>`,
    `    <span class="error-location">${error.file}:${error.line}:${error.column}</span>`,
    `  </div>`,
    `  <div class="error-message">${escapeHtml(error.message)}</div>`,
  ]

  if (error.snippet) {
    parts.push(
      `  <div class="error-snippet">`,
      `    <pre><code>${escapeHtml(error.snippet)}</code></pre>`,
      `    <div class="error-pointer">${' '.repeat(error.column - 1)}^</div>`,
      `  </div>`,
    )
  }

  parts.push(`</div>`)
  return parts.join('\n')
}

export function formatErrorText(error: ParseError) {
  const lines = [
    `${error.type} at ${error.file}:${error.line}:${error.column}`,
    error.message,
  ]

  if (error.snippet) {
    lines.push('', error.snippet)
    lines.push(' '.repeat(error.column - 1) + '^')
  }

  return lines.join('\n')
}

/**
 * Convert ANSI colored text to HTML with inline styles
 */
export function ansiToHtml(str: string): string {
  const colorMap: Record<string, string> = {
    '30': 'color: #000000', // black
    '31': 'color: #cc0000', // red
    '32': 'color: #4e9a06', // green
    '33': 'color: #c4a000', // yellow
    '34': 'color: #3465a4', // blue
    '35': 'color: #75507b', // magenta
    '36': 'color: #06989a', // cyan
    '37': 'color: #d3d7cf', // white
    '90': 'color: #555753', // bright black
    '91': 'color: #ef2929', // bright red
    '92': 'color: #8ae234', // bright green
    '93': 'color: #fce94f', // bright yellow
    '94': 'color: #729fcf', // bright blue
    '95': 'color: #ad7fa8', // bright magenta
    '96': 'color: #34e2e2', // bright cyan
    '97': 'color: #eeeeec', // bright white
  }

  let result = escapeHtml(str)
  let openTags = 0

  result = result.replace(/\x1b\[(\d+(?:;\d+)*)m/g, (match, codes) => {
    const codeList = codes.split(';')

    if (codes === '0' || codes === '') {
      // Reset
      const closeTags = '</span>'.repeat(openTags)
      openTags = 0
      return closeTags
    }

    const styles = codeList
      .map((code: string) => colorMap[code])
      .filter(Boolean)
      .join('; ')

    if (styles) {
      openTags++
      return `<span style="${styles}">`
    }

    return ''
  })

  // Close any remaining open tags
  result += '</span>'.repeat(openTags)

  return `<pre class="ansi-output">${result}</pre>`
}

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

/**
 * Main error handler - use this in your app
 */
export function handleFormatError(error: any): {
  parsed: ParseError
  text: string
  html: string
  ansiHtml: string
} {
  // Extract error string from various error formats
  const errorStr = typeof error === 'string'
    ? error
    : error?.error || error?.message || String(error)

  const parsed = parseFormatError(errorStr)

  return {
    parsed,
    text: formatErrorText(parsed),
    html: formatErrorHtml(parsed),
    ansiHtml: ansiToHtml(errorStr),
  }
}

// CSS styles to include in your app
export const errorStyles = `
  .format-error {
    font-family: 'SF Mono', Monaco, 'Cascadia Code', monospace;
    background: #fee;
    border: 1px solid #fcc;
    border-radius: 4px;
    padding: 12px;
    margin: 8px 0;
  }
  
  .error-header {
    display: flex;
    justify-content: space-between;
    margin-bottom: 8px;
    font-weight: 600;
  }
  
  .error-type {
    color: #d00;
  }
  
  .error-location {
    color: #666;
    font-size: 0.9em;
  }
  
  .error-message {
    color: #333;
    margin-bottom: 8px;
  }
  
  .error-snippet {
    background: #fff;
    border: 1px solid #ddd;
    border-radius: 3px;
    padding: 8px;
    margin-top: 8px;
  }
  
  .error-snippet pre {
    margin: 0;
    overflow-x: auto;
  }
  
  .error-pointer {
    color: #d00;
    font-weight: bold;
  }
  
  .ansi-output {
    background: #1e1e1e;
    color: #d4d4d4;
    padding: 12px;
    border-radius: 4px;
    overflow-x: auto;
    font-family: 'SF Mono', Monaco, 'Cascadia Code', monospace;
    font-size: 13px;
    line-height: 1.4;
  }
`
