#!/usr/bin/env node

import { BINARY_NAME, colors, KNOWN_TOOLS, PLATFORM_SPECIFIC_PACKAGE_NAME, resolveTargetTool } from '#const.mjs'
import * as NodeChildProcess from 'node:child_process'
import * as NodeFS from 'node:fs'
import * as NodeModule from 'node:module'
import * as NodePath from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * @typedef {import('#const.mjs').Tool} Tool
 */

const require = NodeModule.createRequire(import.meta.url)
const __dirname = NodePath.dirname(fileURLToPath(import.meta.url))

const targetTool = resolveTool()
process.env.TARGET_TOOL ??= targetTool

const binaryName = BINARY_NAME(targetTool)
const platformPackage = PLATFORM_SPECIFIC_PACKAGE_NAME(targetTool)

if (!platformPackage) {
  console.error(colors.red, 'Platform not supported!')
  console.error(colors.reset)
  console.error(colors.yellow, `Platform: ${process.platform}, Architecture: ${process.arch}`)
  console.error(colors.reset)
  process.exit(1)
}

const child = NodeChildProcess.spawn(
  selectBinaryPath(),
  process.argv.slice(2),
  { stdio: 'inherit' }
)

/**
 * @type {Record<'SIGINT' | 'SIGTERM', () => void>}
 */
const signalHandlers = {
  SIGINT: () => forwardSignal('SIGINT'),
  SIGTERM: () => forwardSignal('SIGTERM')
}

for (const [signal, handler] of Object.entries(signalHandlers))
  process.on(signal, handler)

/**
 * Determines which tool wrapper is executing.
 * @returns {Tool}
 */
function resolveTool() {
  const candidates = [
    process.env.TARGET_TOOL,
    toolFromPackageName(process.env.npm_package_name),
    toolFromLocalPackage(),
    toolFromPath()
  ]

  for (const candidate of candidates) {
    if (!candidate) continue
    try {
      return resolveTargetTool(candidate)
    } catch {
      // try next
    }
  }

  throw new Error('TARGET_TOOL must be set to one of: ' + KNOWN_TOOLS.join(', '))
}

/**
 * Attempts to read the tool name from the nearest package.json.
 * @returns {Tool | undefined}
 */
function toolFromLocalPackage() {
  try {
    const packageJsonPath = NodePath.join(__dirname, 'package.json')
    if (!NodeFS.existsSync(packageJsonPath)) return undefined
    const pkg = require(packageJsonPath)
    return toolFromPackageName(pkg?.name)
  } catch {
    return undefined
  }
}

/**
 * Extracts the tool name from an @foundry-rs scoped package identifier.
 * @param {unknown} name
 * @returns {Tool | undefined}
 */
function toolFromPackageName(name) {
  if (typeof name !== 'string') return undefined
  const match = name.match(/^@foundry-rs\/(forge|cast|anvil|chisel)(?:$|-)/)
  return match ? /** @type {Tool} */ (match[1]) : undefined
}

/**
 * Walks up the directory tree to infer the tool name from the folder structure.
 * @returns {Tool | undefined}
 */
function toolFromPath() {
  const segments = NodePath.resolve(__dirname).split(NodePath.sep)
  for (let i = segments.length - 1; i >= 0; i--) {
    const candidate = segments[i]
    if (isTool(candidate)) return candidate
  }
  return undefined
}

/**
 * Type guard verifying a candidate string is a known tool.
 * @param {string | undefined} candidate
 * @returns {candidate is Tool}
 */
function isTool(candidate) {
  if (typeof candidate !== 'string') return false
  return KNOWN_TOOLS.includes(/** @type {Tool} */ (candidate))
}

/**
 * Determines the executable file path for the current platform.
 * @returns {string}
 */
function selectBinaryPath() {
  try {
    const candidate = require.resolve(`${platformPackage}/bin/${binaryName}`)
    if (NodeFS.existsSync(candidate)) return candidate
  } catch {
    // fall through to dist/ binary
  }

  return NodePath.join(__dirname, '..', 'dist', binaryName)
}

/**
 * Forwards a received signal to the child process, then re-emits it locally to
 * preserve Node.js default exit semantics.
 * @param {'SIGINT' | 'SIGTERM'} signal
 */
function forwardSignal(signal) {
  try {
    if (!child.killed)
      child.kill(signal)
  } catch (error) {
    if (!error || (typeof error === 'object' && 'code' in error && error.code !== 'ESRCH'))
      throw error
  }

  process.off(signal, signalHandlers[signal])
  process.kill(process.pid, signal)
}
