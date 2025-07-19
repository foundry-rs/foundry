import packageJSON from '#package.json' with { type: 'json' }
import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'
import type * as Process from 'node:process'

export function getRegistryUrl() {
  if (process.env.NODE_ENV !== 'production') return process.env.REGISTRY_URL ?? 'https://registry.npmjs.org'

  return 'https://registry.npmjs.org'
}

export const BINARY_DISTRIBUTION_VERSION = packageJSON.version

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
  'darwin-x64': {
    cpu: 'x64',
    os: 'darwin',
    arch: 'amd64',
    reference: referenceMap['darwin-x64'],
    path: 'forge-darwin-x64',
    get name() {
      return `@foundry-rs/${this.path}`
    },
    get version() {
      return JSON.parse(
        NodeFS.readFileSync(
          NodePath.join(
            NodePath.join(import.meta.dirname, '..', this.name),
            'package.json'
          ),
          { encoding: 'utf-8' }
        )
      ).version
    }
  },
  'darwin-arm64': {
    cpu: 'arm64',
    os: 'darwin',
    arch: 'arm64',
    reference: referenceMap['darwin-arm64'],
    path: 'forge-darwin-arm64',
    get name() {
      return `@foundry-rs/${this.path}`
    },
    get version() {
      return JSON.parse(
        NodeFS.readFileSync(
          NodePath.join(
            NodePath.join(import.meta.dirname, '..', this.name),
            'package.json'
          ),
          { encoding: 'utf-8' }
        )
      ).version
    }
  },
  'linux-x64': {
    cpu: 'x64',
    os: 'linux',
    arch: 'amd64',
    reference: referenceMap['linux-x64'],
    path: 'forge-linux-x64',
    get name() {
      return `@foundry-rs/${this.path}`
    },
    get version() {
      return JSON.parse(
        NodeFS.readFileSync(
          NodePath.join(
            NodePath.join(import.meta.dirname, '..', this.name),
            'package.json'
          ),
          { encoding: 'utf-8' }
        )
      ).version
    }
  },
  'linux-arm64': {
    cpu: 'arm64',
    os: 'linux',
    arch: 'arm64',
    reference: referenceMap['linux-arm64'],
    path: 'forge-linux-arm64',
    get name() {
      return `@foundry-rs/${this.path}`
    },
    get version() {
      return JSON.parse(
        NodeFS.readFileSync(
          NodePath.join(
            NodePath.join(import.meta.dirname, '..', this.name),
            'package.json'
          ),
          { encoding: 'utf-8' }
        )
      ).version
    }
  },
  'win32-x64': {
    cpu: 'x64',
    os: 'win32',
    arch: 'amd64',
    reference: referenceMap['win32-x64'],
    path: 'forge-win32-x64',
    get name() {
      return `@foundry-rs/${this.path}`
    },
    get version() {
      return JSON.parse(
        NodeFS.readFileSync(
          NodePath.join(
            NodePath.join(import.meta.dirname, '..', this.name),
            'package.json'
          ),
          { encoding: 'utf-8' }
        )
      ).version
    }
  }
} as const satisfies Record<
  ArchitecturePlatform,
  {
    version: string
    cpu: string
    os: string
    arch: string
    reference: string
    path: string
    name: string
  }
>

export const BINARY_NAME = process.platform === 'win32' ? 'forge.exe' : 'forge'

export const PLATFORM_SPECIFIC_PACKAGE_NAME = BINARY_DISTRIBUTION_PACKAGES[
  `${process.platform as Platform}-${process.arch as Architecture}` as ArchitecturePlatform
]
