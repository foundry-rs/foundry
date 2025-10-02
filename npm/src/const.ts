import type * as Process from 'node:process'

export function getRegistryUrl() {
  // Prefer npm's configured registry (works with Verdaccio and custom registries)
  // Fallback to REGISTRY_URL for tests/dev, then npmjs
  return (
    process.env.npm_config_registry
    || process.env.REGISTRY_URL
    || 'https://registry.npmjs.org'
  )
}

export type Architecture = Extract<(typeof Process)['arch'], 'arm64' | 'x64'>
export type Platform = Extract<
  (typeof Process)['platform'],
  'darwin' | 'linux' | 'win32'
>

/**
 * foundry doesn't ship arm64 binaries for windows
 */
export type ArchitecturePlatform = Exclude<
  `${Platform}-${Architecture}`,
  'win32-arm64'
>

export const BINARY_DISTRIBUTION_PACKAGES = {
  darwin: {
    x64: '@foundry-rs/forge-darwin-amd64',
    arm64: '@foundry-rs/forge-darwin-arm64'
  },
  linux: {
    x64: '@foundry-rs/forge-linux-amd64',
    arm64: '@foundry-rs/forge-linux-arm64'
  },
  win32: {
    x64: '@foundry-rs/forge-win32-amd64'
  }
} as const

export const BINARY_NAME = process.platform === 'win32' ? 'forge.exe' : 'forge'
// @ts-expect-error
export const PLATFORM_SPECIFIC_PACKAGE_NAME = BINARY_DISTRIBUTION_PACKAGES[process.platform][process.arch]

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
