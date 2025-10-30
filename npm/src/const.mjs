import * as NodePath from 'node:path'

/**
 * @typedef {'amd64' | 'arm64'} Arch
 * @typedef {'linux' | 'darwin' | 'win32'} Platform
 * @typedef {'forge' | 'cast' | 'anvil' | 'chisel'} Tool
 * @typedef {'debug' | 'release' | 'maxperf'} Profile
 */

/** @type {readonly Tool[]} */
export const KNOWN_TOOLS = Object.freeze(['forge', 'cast', 'anvil', 'chisel'])

const TOOL_SET = new Set(KNOWN_TOOLS)

/**
 * @param {string | undefined} [raw]
 * @returns {Tool}
 *
 * could be process.argv[2]
 */
export function resolveTargetTool(raw = process.env.TARGET_TOOL || process.argv[2]) {
  const value = typeof raw === 'string' ? raw.trim() : ''
  if (!value)
    throw new Error('TARGET_TOOL must be set to one of: ' + KNOWN_TOOLS.join(', '))
  if (value !== NodePath.basename(value) || value.includes('..') || value.includes('/') || value.includes('\\'))
    throw new Error('TARGET_TOOL contains invalid path segments')
  // @ts-expect-error _
  if (!TOOL_SET.has(value))
    throw new Error(`TARGET_TOOL "${value}" is not supported. Expected: ${KNOWN_TOOLS.join(', ')}`)
  return /** @type {Tool} */ (value)
}

export function getRegistryUrl() {
  // Prefer npm's configured registry (works with Verdaccio and custom registries)
  // Fallback to REGISTRY_URL for tests/dev, then npmjs
  return (
    process.env.npm_config_registry
    || process.env.REGISTRY_URL
    || 'https://registry.npmjs.org'
  )
}

/**
 * @param {Tool} tool
 * @returns {Record<Platform, Record<string, string>>}
 */
export const BINARY_DISTRIBUTION_PACKAGES = tool => ({
  darwin: {
    x64: `@foundry-rs/${tool}-darwin-amd64`,
    arm64: `@foundry-rs/${tool}-darwin-arm64`
  },
  linux: {
    x64: `@foundry-rs/${tool}-linux-amd64`,
    arm64: `@foundry-rs/${tool}-linux-arm64`
  },
  win32: {
    x64: `@foundry-rs/${tool}-win32-amd64`
  }
})

/**
 * @param {Tool} tool
 * @returns {string}
 */
export const BINARY_NAME = tool => process.platform === 'win32' ? `${tool}.exe` : tool

/**
 * @param {Tool} tool
 * @returns {string | undefined}
 */
export const PLATFORM_SPECIFIC_PACKAGE_NAME = tool => {
  // @ts-ignore
  const platformPackages = BINARY_DISTRIBUTION_PACKAGES(tool)[process.platform]
  if (!platformPackages) return undefined
  return platformPackages?.[process.arch]
}

export const colors = {
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m',
  cyan: '\x1b[36m',
  white: '\x1b[37m',
  reset: '\x1b[0m'
}
