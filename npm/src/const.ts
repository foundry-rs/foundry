import type * as Process from 'node:process'

export function getRegistryUrl() {
  if (process.env.NODE_ENV !== 'production') return process.env.REGISTRY_URL ?? 'https://registry.npmjs.org'

  return 'https://registry.npmjs.org'
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

const referenceMap = {
  'darwin-x64': 'x86_64-apple-darwin',
  'darwin-arm64': 'aarch64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
  'linux-arm64': 'aarch64-unknown-linux-gnu',
  'win32-x64': 'x86_64-pc-windows-msvc'
} as const satisfies Record<ArchitecturePlatform, string>

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
    x64: '@foundry-rs/forge-win32-amd64',
    arm64: '@foundry-rs/forge-win32-arm64'
  }
}

export const BINARY_NAME = process.platform === 'win32' ? 'forge.exe' : 'forge'
// @ts-expect-error
export const PLATFORM_SPECIFIC_PACKAGE_NAME = BINARY_DISTRIBUTION_PACKAGES[process.platform][process.arch]
