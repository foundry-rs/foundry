#!/usr/bin/env bun

import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'
import * as NodeUtil from 'node:util'
import { colors } from '../src/utilities.ts'

const PRESERVED_FILES = ['package.json', 'README.md']
const PLATFORM_MAP = {
  linux: 'linux',
  alpine: 'linux',
  darwin: 'darwin',
  win32: 'win32'
} as const

const TARGET_MAP = {
  'amd64-linux': 'x86_64-unknown-linux-gnu',
  'amd64-alpine': 'x86_64-unknown-linux-musl',
  'arm64-linux': 'aarch64-unknown-linux-gnu',
  'arm64-alpine': 'aarch64-unknown-linux-musl',
  'amd64-darwin': 'x86_64-apple-darwin',
  'arm64-darwin': 'aarch64-apple-darwin',
  'amd64-win32': 'x86_64-pc-windows-msvc'
} as const

const ARCH_MAP = { amd64: 'x64', arm64: 'arm64', aarch64: 'arm64' } as const

main().catch(error => {
  console.error(colors.red, error)
  process.exit(1)
})

async function main() {
  const { platform, arch, forgeBinPath } = getPlatformInfo()
  const distribution = `${platform}-${arch}`
  const packagePath = NodePath.join(process.cwd(), '@foundry-rs', `forge-${distribution}`)

  await cleanPackageDirectory(packagePath)
  await buildScripts()
  await copyBinary(forgeBinPath, packagePath, platform)

  console.info(colors.green, 'Binary copy completed successfully!', colors.reset)
}

function getPlatformInfo() {
  const platform = Bun.env.PLATFORM_NAME as keyof typeof PLATFORM_MAP
  const arch = Bun.env.ARCH as keyof typeof ARCH_MAP
  
  if (!platform || !arch) {
    throw new Error('PLATFORM_NAME and ARCH environment variables are required')
  }

  const npmPlatform = PLATFORM_MAP[platform]
  const npmArch = ARCH_MAP[arch]
  
  if (!npmPlatform || !npmArch) {
    throw new Error('Invalid platform or architecture')
  }

  const { values } = NodeUtil.parseArgs({
    args: Bun.argv,
    strict: true,
    allowPositionals: true,
    options: {
      'forge-bin-path': { type: 'string', default: Bun.env.FORGE_BIN_PATH }
    }
  })

  const profile = Bun.env.NODE_ENV === 'production' ? 'release' : Bun.env.PROFILE || 'release'
  const forgeBinPath = values['forge-bin-path'] || findForgeBinary(arch, platform, profile)

  return { platform: npmPlatform, arch: npmArch, forgeBinPath }
}

function findForgeBinary(arch: string, platform: string, profile: string): string {
  const targetDir = TARGET_MAP[`${arch}-${platform}` as keyof typeof TARGET_MAP]
  const targetPath = NodePath.join(process.cwd(), '..', 'target', targetDir, profile, 'forge')
  
  if (NodeFS.existsSync(targetPath)) {
    return targetPath
  }
  
  return NodePath.join(process.cwd(), '..', 'target', 'release', 'forge')
}

async function cleanPackageDirectory(packagePath: string) {
  const items = await NodeFS.promises.readdir(packagePath, {
    withFileTypes: true,
    recursive: true
  })
  
  items
    .filter(item => !PRESERVED_FILES.includes(item.name))
    .forEach(item => NodeFS.rmSync(NodePath.join(packagePath, item.name), {
      recursive: true,
      force: true
    }))

  console.info(colors.green, 'Cleaned up package directory', colors.reset)
}

async function buildScripts() {
  const result = await Bun.$`bun x tsdown --config tsdown.config.ts`.nothrow().quiet()
  
  if (result.exitCode !== 0) {
    throw new Error(`Failed to build scripts: ${result.stderr.toString()}`)
  }
  
  console.info(colors.green, result.stdout.toString(), colors.reset)
}

async function copyBinary(forgeBinPath: string, packagePath: string, platform: string) {
  if (!(await Bun.file(forgeBinPath).exists())) {
    throw new Error(`Source binary not found at ${forgeBinPath}`)
  }

  const binaryName = platform === 'win32' ? 'forge.exe' : 'forge'
  const targetDir = NodePath.join('@foundry-rs', NodePath.basename(packagePath), 'bin')
  
  NodeFS.mkdirSync(targetDir, { recursive: true })
  
  const targetPath = NodePath.join(targetDir, binaryName)
  console.info(colors.green, `Copying ${forgeBinPath} to ${targetPath}`, colors.reset)
  
  await Bun.write(targetPath, Bun.file(forgeBinPath))
  
  if (platform !== 'win32') {
    NodeFS.chmodSync(targetPath, 0o755)
  }
}
