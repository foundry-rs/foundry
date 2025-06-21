#!/usr/bin/env bun

import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'
import * as NodeUtil from 'node:util'
import * as Bun from 'bun'
import { colors } from '../src/utilities.ts'

const nonGeneratedArtifacts = ['package.json', 'README.md']

const platformMap = {
  linux: 'linux',
  alpine: 'linux', // alpine uses linux in npm package names
  darwin: 'darwin',
  win32: 'win32'
} as const

const archMap = {
  amd64: 'x64',
  arm64: 'arm64',
  aarch64: 'arm64'
} as const

main().catch((error) => {
  console.error(colors.red, error)
  process.exit(1)
})

async function main() {
  const platform = Bun.env.PLATFORM_NAME as keyof typeof platformMap
  const arch = Bun.env.ARCH as keyof typeof archMap
  const profile =
    Bun.env.NODE_ENV === 'production' ? 'release' : Bun.env.PROFILE || 'release'

  if (!platform)
    throw new Error('PLATFORM_NAME environment variable is not set')
  if (!arch) throw new Error('ARCH environment variable is not set')

  const [npmPlatform, npmArch] = [platformMap[platform], archMap[arch]]

  if (!npmPlatform || !npmArch)
    throw new Error('Invalid platform or architecture')

  const packageDir = `${npmPlatform}-${npmArch}`

  const { values } = NodeUtil.parseArgs({
    args: Bun.argv,
    strict: true,
    allowPositionals: true,
    options: {
      arch: { type: 'string', default: arch },
      target: { type: 'string', default: Bun.env.TARGET },
      'forge-bin-path': { type: 'string', default: Bun.env.FORGE_BIN_PATH }
    }
  })

  // `darwin-arm64`, `darwin-x64`, `linux-arm64`, `linux-x64`, `win32-arm64`, `win32-x64`
  const distribution = `${npmPlatform}-${npmArch}`
  if (!distribution) throw new Error('Distribution is required')

  const packagePath = NodePath.join(
    process.cwd(),
    '@foundry-rs',
    `forge-${distribution}`
  )

  const directoryItems = await NodeFS.promises.readdir(packagePath, {
    withFileTypes: true,
    recursive: true
  })
  directoryItems
    .filter((item) => !nonGeneratedArtifacts.includes(item.name))
    .forEach((item) =>
      NodeFS.rmSync(NodePath.join(packagePath, item.name), {
        recursive: true,
        force: true
      })
    )

  console.info(colors.green, 'Cleaned up package directory', colors.reset)

  // Use the forge_bin_path from GitHub Actions if available, otherwise construct it
  const forgeBinPath =
    values['forge-bin-path'] ||
    (values.target
      ? `../target/${values.target}/${profile}/forge`
      : `../target/${values.arch}/${profile}/forge`)

  if (!(await Bun.file(forgeBinPath).exists()))
    throw new Error(`Source binary not found at ${forgeBinPath}`)

  const buildScripts = await Bun.$`bun x tsdown --config tsdown.config.ts`
    .nothrow()
    .quiet()

  if (buildScripts.exitCode !== 0)
    throw new Error(
      `Failed to build scripts: ${buildScripts.stderr.toString()}`
    )

  console.info(colors.green, buildScripts.stdout.toString(), colors.reset)

  // Determine binary name (add .exe for Windows)
  const binaryName = platform === 'win32' ? 'forge.exe' : 'forge'

  // Copy to npm/@foundry-rs/forge-{platform}-{arch}/bin/
  const forgePackageDir = NodePath.join(
    '@foundry-rs',
    `forge-${packageDir}`,
    'bin'
  )

  if (!(await Bun.file(forgePackageDir).exists()))
    NodeFS.mkdirSync(forgePackageDir, { recursive: true })

  const targetPath = NodePath.join(forgePackageDir, binaryName)

  if (!(await Bun.file(forgeBinPath).exists()))
    throw new Error(`Source binary not found at ${forgeBinPath}`)

  console.info(
    colors.green,
    `Copying ${forgeBinPath} to ${targetPath}`,
    colors.reset
  )
  await Bun.write(targetPath, Bun.file(forgeBinPath))

  // Make binary executable on Unix-like systems
  if (platform !== 'win32') NodeFS.chmodSync(targetPath, 0o755)

  console.info(
    colors.green,
    'Binary copy completed successfully!',
    colors.reset
  )
}
