#!/usr/bin/env bun

import * as NodeFS from 'node:fs/promises'
import * as NodePath from 'node:path'
import * as NodeUtil from 'node:util'

import { colors } from '#const.mjs'

/**
 * @typedef {import('#const.mjs').Tool} Tool
 * @typedef {import('#const.mjs').Arch} Arch
 * @typedef {import('#const.mjs').Platform} Platform
 * @typedef {{
 *  tool: Tool
 *  platform: Platform
 *  arch: Arch
 *  packagePath: string
 * }} GenerateOptions
 */

const TOOL_META = /** @type {const} */ (/** @type {Record<Tool, { homepage: string; description: string }>} */ ({
  forge: {
    homepage: 'https://getfoundry.sh/forge',
    description: 'Fast and flexible Ethereum testing framework'
  },
  cast: {
    homepage: 'https://getfoundry.sh/cast',
    description: 'Swiss Army knife for interacting with Ethereum applications from the command line'
  },
  anvil: {
    homepage: 'https://getfoundry.sh/anvil',
    description: 'Anvil is a fast local Ethereum development node'
  },
  chisel: {
    homepage: 'https://getfoundry.sh/chisel',
    description: 'Chisel is a fast, utilitarian, and verbose Solidity REPL'
  }
}))

/**
 * @param {GenerateOptions} options
 * @returns {Promise<void>}
 */
export async function generateBinaryPackageJson({
  tool,
  platform,
  arch,
  packagePath
}) {
  const packageJsonPath = NodePath.join(packagePath, 'package.json')

  const cpu = arch === 'amd64' ? 'x64' : 'arm64'
  const isWindows = platform === 'win32'
  const binName = isWindows ? `${tool}.exe` : tool
  const humanPlatform = platform === 'darwin' ? 'macOS' : platform === 'win32' ? 'Windows' : 'Linux'
  const { homepage, description } = TOOL_META[tool]

  const pkg = {
    name: `@foundry-rs/${tool}-${platform}-${arch}`,
    version: '0.0.0',
    type: 'module',
    homepage,
    description: `${description} (${humanPlatform} ${cpu})`,
    bin: { [tool]: `./bin/${binName}` },
    os: [platform],
    cpu: [cpu],
    files: ['bin'],
    engines: { node: '>=20' },
    license: 'MIT OR Apache-2.0',
    repository: {
      directory: 'npm',
      url: 'https://github.com/foundry-rs/foundry'
    },
    keywords: ['foundry', 'testing', 'ethereum', 'solidity', 'blockchain', 'smart-contracts'],
    publishConfig: { provenance: true }
  }

  await Bun.write(packageJsonPath, JSON.stringify(pkg, null, 2))
  console.info(colors.green, `Wrote ${NodePath.relative(process.cwd(), packageJsonPath)}`, colors.reset)
}

// CLI entrypoint so CI can call directly if desired
if (import.meta.main) {
  const { values } = NodeUtil.parseArgs({
    args: Bun.argv,
    options: {
      tool: { type: 'string', default: Bun.env.TARGET_TOOL },
      platform: { type: 'string' },
      arch: { type: 'string' },
      out: { type: 'string' }
    },
    strict: true
  })

  const tool = /** @type {Tool} */ (String(values.tool || Bun.env.TARGET_TOOL))
  const platform = /** @type {Platform} */ (values.platform || Bun.env.PLATFORM_NAME)
  const arch = /** @type {Arch} */ (values.arch || Bun.env.ARCH)
  const out = /** @type {string} */ (values.out || '')

  if (!platform || !arch)
    throw new Error('platform and arch are required (flags or env PLATFORM_NAME/ARCH)')
  if (!out)
    throw new Error('out is required (path to per-arch package directory)')

  // Ensure the directory exists
  await NodeFS.mkdir(out, { recursive: true })
  await generateBinaryPackageJson({ tool, platform, arch, packagePath: out })
}
