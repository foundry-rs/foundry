#!/usr/bin/env bun

import * as NodeFS from 'node:fs/promises'
import * as NodePath from 'node:path'
import * as NodeUtil from 'node:util'

import { colors, KNOWN_TOOLS } from '#const.mjs'

/**
 * @typedef {import('#const.mjs').Tool} Tool
 */

main().catch(error => {
  console.error(colors.red, error)
  console.error(colors.reset)
  process.exit(1)
})

/**
 * Publish each meta package (`@foundry-rs/<tool>`) to npm.
 * @returns {Promise<void>}
 */
async function main() {
  const { tools, releaseVersion } = resolveArgs()

  for (const tool of tools) {
    await prepareMetaPackage(tool)
    const packageDir = NodePath.join('@foundry-rs', tool)
    console.info(colors.green, `Publishing meta package ${packageDir}`, colors.reset)

    const result = await Bun
      .$`bun ./scripts/publish.mjs ${packageDir}`
      .cwd(NodePath.resolve(import.meta.dir, '..'))
      .env({
        ...process.env,
        TARGET_TOOL: tool,
        TOOL: tool,
        RELEASE_VERSION: releaseVersion,
        VERSION_NAME: releaseVersion
      })
      .nothrow()

    if (result.exitCode !== 0) {
      const stderr = typeof result.stderr === 'string' ? result.stderr : result.stderr?.toString('utf8')
      const stdout = typeof result.stdout === 'string' ? result.stdout : result.stdout?.toString('utf8')
      throw new Error(stderr || stdout || `Failed to publish ${packageDir}`)
    }
  }
}

/**
 * Resolve CLI arguments and defaults.
 * @returns {{ tools: Tool[]; releaseVersion: string }}
 */
function resolveArgs() {
  const { values, positionals } = NodeUtil.parseArgs({
    args: Bun.argv.slice(2),
    allowPositionals: true,
    options: {
      tool: { type: 'string', multiple: true },
      tools: { type: 'string', multiple: true },
      'release-version': { type: 'string' },
      release: { type: 'string' }
    },
    strict: true
  })

  const releaseCandidate = values['release-version']
    || values.release
    || process.env.RELEASE_VERSION
    || process.env.VERSION_NAME
    || ''

  const releaseVersion = releaseCandidate.trim()
  if (!releaseVersion)
    throw new Error('Missing required RELEASE_VERSION')

  const explicitTools = [...(values.tool ?? []), ...(values.tools ?? []), ...positionals]

  const selected = explicitTools.length ? explicitTools : KNOWN_TOOLS
  const tools = /** @type {Tool[]} */ (selected.map(candidate => {
    const trimmed = candidate.trim()
    if (!trimmed) return undefined
    const maybeTool = /** @type {Tool} */ (trimmed)
    if (!KNOWN_TOOLS.includes(maybeTool))
      throw new Error(`Unsupported tool: ${candidate}`)
    return maybeTool
  }).filter(Boolean))

  return { tools, releaseVersion }
}

/**
 * Ensure the meta package directory contains the runtime files.
 * @param {Tool} tool
 * @returns {Promise<void>}
 */
async function prepareMetaPackage(tool) {
  const npmDir = NodePath.resolve(import.meta.dir, '..')
  const sourceDir = NodePath.join(npmDir, 'src')
  const metaDir = NodePath.join(npmDir, '@foundry-rs', tool)
  await NodeFS.rm(NodePath.join(metaDir, 'dist'), { recursive: true, force: true })

  const postinstallPath = NodePath.join(metaDir, 'postinstall.mjs')
  const buildResult = await Bun
    .$`bun build ${NodePath.join(sourceDir, 'install.mjs')} --format esm --outfile ${postinstallPath} --target node`
    .cwd(npmDir)
    .nothrow()

  if (buildResult.exitCode !== 0) {
    const stderr = typeof buildResult.stderr === 'string'
      ? buildResult.stderr
      : buildResult.stderr?.toString('utf8')
    const stdout = typeof buildResult.stdout === 'string'
      ? buildResult.stdout
      : buildResult.stdout?.toString('utf8')
    throw new Error(stderr || stdout || 'Failed to build postinstall script')
  }

  const binSource = await NodeFS.readFile(NodePath.join(sourceDir, 'bin.mjs'), 'utf8')
  await NodeFS.writeFile(NodePath.join(metaDir, 'bin.mjs'), binSource)

  const constSource = await NodeFS.readFile(NodePath.join(sourceDir, 'const.mjs'), 'utf8')
  await NodeFS.writeFile(NodePath.join(metaDir, 'const.mjs'), constSource)

  const packageJsonPath = NodePath.join(metaDir, 'package.json')
  const pkg = JSON.parse(await NodeFS.readFile(packageJsonPath, 'utf8'))
  pkg.imports = { ...(pkg.imports || {}), '#const.mjs': './const.mjs' }
  pkg.scripts = { ...(pkg.scripts || {}), postinstall: `TARGET_TOOL=${tool} node ./postinstall.mjs` }

  const files = new Set([...(pkg.files ?? [])])
  files.delete('dist')
  files.delete('bin')
  files.add('bin.mjs')
  files.add('const.mjs')
  files.add('postinstall.mjs')
  pkg.files = Array.from(files)

  await NodeFS.writeFile(packageJsonPath, JSON.stringify(pkg, null, 2) + '\n')
}
