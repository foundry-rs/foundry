#!/usr/bin/env bun

import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'

import { colors } from '#const.mjs'

const REGISTRY_URL = Bun.env.NPM_REGISTRY_URL || 'https://registry.npmjs.org'

const NPM_TOKEN = Bun.env.NPM_TOKEN
if (!NPM_TOKEN) throw new Error('NPM_TOKEN is required')

main().catch(error => {
  console.error(error)
  process.exit(1)
})

async function main() {
  const npmToken = Bun.env.NPM_TOKEN
  if (!npmToken) throw new Error('NPM_TOKEN is required')

  const inputPath = Bun.argv[2]
  if (!inputPath) throw new Error('Package path is required')
  const packagePath = NodePath.resolve(inputPath)
  console.info(colors.green, 'Package path:', packagePath)

  const publishVersion = getPublishVersion()
  console.info(colors.green, 'Publish version:', publishVersion)
  if (!publishVersion) throw new Error('Publish version is required')

  if (await isMetaPackage(packagePath))
    await updateOptionalDependencies(packagePath, publishVersion)

  await setPackageVersion(packagePath, publishVersion)
  const packedFile = await packPackage(packagePath)
  await publishPackage(packagePath, packedFile, publishVersion)
}

/**
 * @returns {string}
 */
function getPublishVersion() {
  const maybeVersion = (Bun.env.VERSION_NAME || '').replace(/^v/, '')
  if (maybeVersion && Bun.semver.satisfies(maybeVersion, maybeVersion)) return maybeVersion

  const bump = (Bun.env.BUMP_VERSION || '').replace(/^v/, '')
  if (bump && (!!bump && Bun.semver.satisfies(bump, bump))) return bump

  const releaseVersion = (Bun.env.RELEASE_VERSION || '').replace(/^v/, '')
  const isNightly = releaseVersion.toLowerCase() === 'nightly' || Bun.env.IS_NIGHTLY === 'true'

  const cargoToml = NodeFS.readFileSync(
    NodePath.join(import.meta.dirname, '..', '..', 'Cargo.toml'),
    'utf-8'
  )

  const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)
  if (!versionMatch) throw new Error('Version not found in Cargo.toml')

  const [, base] = versionMatch
  if (!base) throw new Error('Version not found in Cargo.toml')
  if (!isNightly) return base

  const date = new Date()
  const y = date.getUTCFullYear()
  const m = String(date.getUTCMonth() + 1).padStart(2, '0')
  const d = String(date.getUTCDate()).padStart(2, '0')
  const yyyymmdd = `${y}${m}${d}`
  const sha = (Bun.env.GITHUB_SHA || '').slice(0, 7)
  const suffix = sha ? `nightly.${yyyymmdd}.${sha}` : `nightly.${yyyymmdd}`
  return `${base}-${suffix}`
}

/**
 * @param {string} packagePath
 * @param {string} version
 * @returns {Promise<void>}
 */
async function updateOptionalDependencies(packagePath, version) {
  const packageJsonPath = NodePath.join(packagePath, 'package.json')
  const packageJson = JSON.parse(NodeFS.readFileSync(packageJsonPath, 'utf-8'))

  if (packageJson.optionalDependencies) {
    Object.keys(packageJson.optionalDependencies).forEach(key => {
      packageJson.optionalDependencies[key] = version
    })

    await Bun.write(packageJsonPath, JSON.stringify(packageJson, null, 2))
  }
}

/**
 * @param {string} packagePath
 * @returns {Promise<boolean>}
 */
async function isMetaPackage(packagePath) {
  try {
    const packageJsonPath = NodePath.join(packagePath, 'package.json')
    const packageJson = JSON.parse(NodeFS.readFileSync(packageJsonPath, 'utf-8'))
    return ['@foundry-rs/forge', '@foundry-rs/cast', '@foundry-rs/anvil', '@foundry-rs/chisel'].includes(
      packageJson?.name
    )
  } catch {
    return false
  }
}

/**
 * @param {string} packagePath
 * @param {string} version
 * @returns {Promise<void>}
 */
async function setPackageVersion(packagePath, version) {
  console.info(colors.green, 'Setting package version:', version)
  const result = await Bun.$`npm version ${version} --allow-same-version --no-git-tag-version`
    .cwd(packagePath)
    .env({
      ...Bun.env,
      ...process.env,
      NPM_TOKEN
    })
    .quiet()
    .nothrow()

  if (result.exitCode !== 0)
    throw new Error(`Failed to set version: ${result.stderr}`)
}

/**
 * @param {string} packagePath
 * @returns {Promise<string>}
 */
async function packPackage(packagePath) {
  let packedFile = ''

  for await (const line of Bun.$`bun pm pack`.cwd(packagePath).lines())
    if (line.endsWith('.tgz')) packedFile = line

  if (!packedFile) throw new Error('Failed to pack package')
  return packedFile
}

/**
 * @param {string} packagePath
 * @param {string} packedFile
 * @param {string} version
 * @returns {Promise<void>}
 */
async function publishPackage(packagePath, packedFile, version) {
  const tag = /-nightly(\.|$)/.test(version) ? 'nightly' : 'latest'
  const result = await Bun
    .$`npm publish ./${packedFile} --access=public --registry=${REGISTRY_URL} --tag=${tag} --provenance=${
    Bun.env.PROVENANCE || 'true'
  }`
    .cwd(packagePath)
    .nothrow()

  if (result.exitCode !== 0)
    throw new Error(`Publish failed: ${result.stderr}`)
}
