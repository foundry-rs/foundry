#!/usr/bin/env bun

import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'
import * as NodeUtil from 'node:util'

import { colors, KNOWN_TOOLS, resolveTargetTool } from '#const.mjs'
import { generateBinaryPackageJson } from '../src/generate-package-json.mjs'

/**
 * @typedef {import('#const.mjs').Arch} Arch
 * @typedef {import('#const.mjs').Platform} Platform
 * @typedef {import('#const.mjs').Profile} Profile
 * @typedef {import('#const.mjs').Tool} Tool
 */

/**
 * @typedef {{
 *   tool: Tool
 *   platform: Platform
 *   arch: Arch
 *   binaryPath: string
 * }} ResolvedInputs
 */

/**
 * @typedef {{
 *   tool: Tool
 *   platform: Platform
 *   arch: Arch
 *   profile: Profile
 *   cliCandidates: Array<string | undefined>
 * }} ResolveBinaryPathOptions
 */

/**
 * @typedef {{
 *   tool: Tool
 *   platform: Platform
 *   arch: Arch
 *   profile: Profile
 * }} FallbackSearchOptions
 */

const PLATFORM_MAP = /** @type {const} */ (/** @type {Record<Platform, Platform>} */ ({
  linux: 'linux',
  darwin: 'darwin',
  win32: 'win32'
}))

const TARGET_MAP = /** @type {const} */ (/** @type {Record<`${Arch}-${Platform}`, string>} */ ({
  'amd64-linux': 'x86_64-unknown-linux-gnu',
  'arm64-linux': 'aarch64-unknown-linux-gnu',
  'amd64-darwin': 'x86_64-apple-darwin',
  'arm64-darwin': 'aarch64-apple-darwin',
  'amd64-win32': 'x86_64-pc-windows-msvc'
}))

const PRESERVE = new Set(['package.json', 'README.md'])
const GENERIC_BIN_ENV_KEYS = [
  'BIN_PATH',
  'bin_path',
  'BIN',
  'BINARY_PATH',
  'binary_path',
  'TARGET_BIN_PATH',
  'target_bin_path',
  'TARGET_BINARY_PATH',
  'target_binary_path'
]

const TOOL_ENV_KEYS = /** @type {Record<Tool, readonly string[]>} */ ({
  forge: ['forge_bin_path', 'FORGE_BIN_PATH'],
  cast: ['cast_bin_path', 'CAST_BIN_PATH'],
  anvil: ['anvil_bin_path', 'ANVIL_BIN_PATH'],
  chisel: ['chisel_bin_path', 'CHISEL_BIN_PATH']
})

main().catch(error => {
  console.error(colors.red, error)
  process.exit(1)
})

/**
 * Orchestrates package preparation for the current platform/tool pair.
 * @returns {Promise<void>}
 */
async function main() {
  const { tool, platform, arch, binaryPath } = resolveInputs()
  const packagePath = NodePath.join(process.cwd(), '@foundry-rs', `${tool}-${platform}-${arch}`)

  await NodeFS.promises.mkdir(packagePath, { recursive: true, mode: 0o755 })
  console.info(colors.green, `Ensured package directory at ${packagePath}`, colors.reset)

  await generateBinaryPackageJson({ tool, platform, arch, packagePath })
  await cleanPackageDirectory(packagePath)
  await copyBinary({ source: binaryPath, tool, packagePath, platform })

  console.info(colors.green, 'Binary copy completed successfully!', colors.reset)
}

/**
 * Collects CLI/env inputs, normalises them, and resolves the binary path.
 * @returns {ResolvedInputs}
 */
function resolveInputs() {
  const parsed = NodeUtil.parseArgs({
    args: Bun.argv.slice(2),
    allowPositionals: true,
    options: {
      tool: { type: 'string' },
      binary: { type: 'string' },
      bin: { type: 'string' },
      'bin-path': { type: 'string' },
      'binary-path': { type: 'string' }
    },
    strict: true
  })

  const platformEnv = Bun.env.PLATFORM_NAME || ''
  const archEnv = Bun.env.ARCH || ''

  const platform = PLATFORM_MAP[platformEnv]
  const arch = archEnv === 'aarch64' ? 'arm64' : archEnv

  if (!platform || (arch !== 'amd64' && arch !== 'arm64'))
    throw new Error(`Invalid platform or architecture: platform=${platformEnv}, arch=${archEnv}`)

  const tool = resolveTool([
    parsed.values.tool,
    parsed.positionals[0],
    Bun.env.TARGET_TOOL,
    Bun.env.TOOL,
    Bun.env.BINARY_TOOL
  ])

  const profile = Bun.env.NODE_ENV === 'production' ? 'release' : Bun.env.PROFILE || 'release'
  const binaryPath = resolveBinaryPath({
    tool,
    platform,
    arch,
    profile,
    cliCandidates: [
      parsed.values.binary,
      parsed.values.bin,
      parsed.values['bin-path'],
      parsed.values['binary-path'],
      parsed.positionals[1]
    ]
  })

  return { tool, platform, arch, binaryPath }
}

/**
 * Picks the first candidate that resolves to a known tool.
 * @param {Array<string | undefined>} candidates
 * @returns {Tool}
 */
function resolveTool(candidates) {
  for (const candidate of candidates) {
    if (typeof candidate !== 'string' || candidate.trim() === '') continue
    try {
      return resolveTargetTool(candidate)
    } catch {
      // try the next candidate
    }
  }

  throw new Error(`Tool not specified. Provide --tool=<${KNOWN_TOOLS.join('|')}> or set TARGET_TOOL.`)
}

/**
 * Resolves the filesystem path to the tool binary, honouring CLI/env overrides.
 * @param {ResolveBinaryPathOptions} options
 * @returns {string}
 */
function resolveBinaryPath({ tool, platform, arch, profile, cliCandidates }) {
  const envCandidates = [
    ...GENERIC_BIN_ENV_KEYS,
    ...(TOOL_ENV_KEYS[tool] ?? [])
  ].map(readEnv)

  for (const candidate of [...cliCandidates, ...envCandidates]) {
    if (typeof candidate === 'string' && candidate.trim())
      return NodePath.resolve(candidate.trim())
  }

  return findBinaryFallback({ tool, platform, arch, profile })
}

/**
 * Searches the local Cargo build artefacts for the requested binary as a fallback.
 * @param {FallbackSearchOptions} options
 * @returns {string}
 */
function findBinaryFallback({ tool, platform, arch, profile }) {
  const targetDir = TARGET_MAP[`${arch}-${platform}`]
  const binaryName = platform === 'win32' ? `${tool}.exe` : tool
  const searchOrder = []

  if (targetDir)
    searchOrder.push(NodePath.join(process.cwd(), '..', 'target', targetDir, profile, binaryName))

  searchOrder.push(
    NodePath.join(process.cwd(), '..', 'target', profile, binaryName),
    NodePath.join(process.cwd(), '..', 'target', 'release', binaryName)
  )

  for (const candidate of searchOrder)
    if (candidate && NodeFS.existsSync(candidate)) return candidate

  throw new Error(`Source binary for ${tool} not found. Looked in: ${searchOrder.join(', ')}`)
}

/**
 * Removes previously staged files while preserving metadata such as README/package.json.
 * @param {string} packagePath
 * @returns {Promise<void>}
 */
async function cleanPackageDirectory(packagePath) {
  const items = await NodeFS.promises.readdir(packagePath).catch(() => [])

  for (const item of items) {
    if (PRESERVE.has(item)) continue
    NodeFS.rmSync(NodePath.join(packagePath, item), { recursive: true, force: true })
  }

  console.info(colors.green, 'Cleaned up package directory', colors.reset)
}

/**
 * Copies the tool binary into the package directory.
 * @param {{ source: string; tool: Tool; packagePath: string; platform: Platform }} parameters
 * @returns {Promise<void>}
 */
async function copyBinary({ source, tool, packagePath, platform }) {
  if (!(await Bun.file(source).exists()))
    throw new Error(`Source binary not found at ${source}`)

  const binaryName = platform === 'win32' ? `${tool}.exe` : tool
  const targetDir = NodePath.join(packagePath, 'bin')

  NodeFS.mkdirSync(targetDir, { recursive: true })

  const targetPath = NodePath.join(targetDir, binaryName)
  console.info(colors.green, `Copying ${source} to ${targetPath}`, colors.reset)

  await Bun.write(targetPath, Bun.file(source))

  if (platform !== 'win32')
    NodeFS.chmodSync(targetPath, 0o755)
}

/**
 * Reads an environment variable, falling back across case variants.
 * @param {string | undefined} key
 * @returns {string | undefined}
 */
function readEnv(key) {
  if (!key) return undefined
  return Bun.env[key] ?? Bun.env[key.toUpperCase()] ?? Bun.env[key.toLowerCase()]
}
