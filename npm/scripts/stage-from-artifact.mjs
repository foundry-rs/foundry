#!/usr/bin/env bun

import * as NodeFS from 'node:fs/promises'
import * as NodeOS from 'node:os'
import * as NodePath from 'node:path'
import * as NodeUtil from 'node:util'

import { colors } from '#const.mjs'

/**
 * @typedef {import('#const.mjs').Tool} Tool
 * @typedef {import('#const.mjs').Platform} Platform
 * @typedef {import('#const.mjs').Arch} Arch
 */

const RELEASE_ARTIFACT_PREFIX = 'foundry'

main().catch(error => {
  console.error(colors.red, error)
  console.error(colors.reset)
  process.exit(1)
})

/**
 * Entry point: locate the platform-specific artifact, extract the binary,
 * and delegate to prepublish to stage it into the package directory.
 * @returns {Promise<void>}
 */
async function main() {
  const { tool, platform, arch, releaseVersion, artifactDir } = resolveArgs()

  const artifactPrefix = NodePath.join(
    artifactDir,
    `${RELEASE_ARTIFACT_PREFIX}_${releaseVersion}_${platform}_${arch}`
  )

  const archivePath = await chooseArchive(artifactPrefix)
  const extractionDir = await extractArchive(archivePath)

  try {
    const binaryPath = await resolveExtractedBinary({ tool, platform, extractionDir })
    await stagePackage({ tool, platform, arch, binaryPath })
  } finally {
    await NodeFS.rm(extractionDir, { recursive: true, force: true })
  }
}

/**
 * Parse CLI arguments/environment.
 * @returns {{ tool: Tool; platform: Platform; arch: Arch; releaseVersion: string; artifactDir: string }}
 */
function resolveArgs() {
  const { values } = NodeUtil.parseArgs({
    args: Bun.argv.slice(2),
    options: {
      tool: { type: 'string' },
      platform: { type: 'string' },
      arch: { type: 'string' },
      release: { type: 'string' },
      'release-version': { type: 'string' },
      artifacts: { type: 'string' },
      'artifact-dir': { type: 'string' }
    },
    strict: true
  })

  const tool = requireValue(values.tool || process.env.TARGET_TOOL, 'tool')
  const platform = requireValue(values.platform || process.env.PLATFORM_NAME, 'platform')
  const arch = requireValue(values.arch || process.env.ARCH, 'arch')
  const releaseVersion = requireValue(
    values.release || values['release-version'] || process.env.RELEASE_VERSION,
    'release version'
  )
  const artifactDir = requireValue(
    values.artifacts || values['artifact-dir'] || process.env.ARTIFACT_DIR,
    'artifact directory'
  )

  return /** @type {{ tool: Tool; platform: Platform; arch: Arch; releaseVersion: string; artifactDir: string }} */ ({
    tool: /** @type {Tool} */ (tool),
    platform: /** @type {Platform} */ (platform),
    arch: /** @type {Arch} */ (arch),
    releaseVersion,
    artifactDir: NodePath.resolve(artifactDir)
  })
}

/**
 * @param {string | undefined} value
 * @param {string} name
 * @returns {string}
 */
function requireValue(value, name) {
  if (typeof value === 'string' && value.trim()) return value.trim()
  throw new Error(`Missing required ${name}`)
}

/**
 * Determine which archive variant exists for the given artifact prefix.
 * @param {string} prefix
 * @returns {Promise<string>}
 */
async function chooseArchive(prefix) {
  const tarPath = `${prefix}.tar.gz`
  const zipPath = `${prefix}.zip`

  if (await pathExists(tarPath)) return tarPath
  if (await pathExists(zipPath)) return zipPath

  throw new Error(`No release artifact found for prefix ${prefix}`)
}

/**
 * @param {string} filePath
 * @returns {Promise<boolean>}
 */
async function pathExists(filePath) {
  try {
    await NodeFS.access(filePath)
    return true
  } catch {
    return false
  }
}

/**
 * Extract the archive into a temporary directory.
 * @param {string} archivePath
 * @returns {Promise<string>}
 */
async function extractArchive(archivePath) {
  const tempDir = await NodeFS.mkdtemp(NodePath.join(NodeOS.tmpdir(), 'foundry-npm-'))
  const command = archivePath.endsWith('.zip')
    ? Bun.$`unzip -o ${archivePath} -d ${tempDir}`
    : Bun.$`tar -xzf ${archivePath} -C ${tempDir}`

  const result = await command
    .env(process.env)
    .nothrow()

  if (result.exitCode !== 0) {
    const stderr = typeof result.stderr === 'string'
      ? result.stderr
      : result.stderr?.toString('utf8')
    const stdout = typeof result.stdout === 'string'
      ? result.stdout
      : result.stdout?.toString('utf8')
    throw new Error(`Failed to extract ${archivePath}: ${stderr || stdout || 'unknown error'}`)
  }

  return tempDir
}

/**
 * Locate the expected binary within the extraction directory.
 * @param {{ tool: Tool; platform: Platform; extractionDir: string }} options
 * @returns {Promise<string>}
 */
async function resolveExtractedBinary({ tool, platform, extractionDir }) {
  const binaryName = platform === 'win32' ? `${tool}.exe` : tool
  const candidate = NodePath.join(extractionDir, binaryName)

  if (await pathExists(candidate)) return candidate

  throw new Error(`Binary ${binaryName} not found in ${extractionDir}`)
}

/**
 * Delegate to prepublish to stage the extracted binary into the package dir.
 * @param {{ tool: Tool; platform: Platform; arch: Arch; binaryPath: string }} options
 * @returns {Promise<void>}
 */
async function stagePackage({ tool, platform, arch, binaryPath }) {
  console.info(colors.green, `Staging ${tool} (${platform}/${arch}) from ${binaryPath}`, colors.reset)

  const subprocess = Bun
    .$`bun ./scripts/prepublish.mjs --tool ${tool} --binary ${binaryPath}`
    .cwd(NodePath.resolve(import.meta.dir, '..'))
    .env({
      ...process.env,
      TARGET_TOOL: tool,
      TOOL: tool,
      PLATFORM_NAME: platform,
      ARCH: arch
    })

  const result = await subprocess.nothrow()
  if (result.exitCode !== 0) {
    const stderr = typeof result.stderr === 'string'
      ? result.stderr
      : result.stderr?.toString('utf8')
    throw new Error(stderr || `Failed to stage package for ${tool}`)
  }
}
