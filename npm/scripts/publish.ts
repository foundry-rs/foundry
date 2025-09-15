#!/usr/bin/env bun
import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'
import { colors } from '../src/const'

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

function getPublishVersion() {
  const maybeVersion = (Bun.env.VERSION_NAME || '').replace(/^v/, '')
  if (maybeVersion && isValidSemver(maybeVersion)) return maybeVersion

  const bump = (Bun.env.BUMP_VERSION || '').replace(/^v/, '')
  if (bump && isValidSemver(bump)) return bump

  const releaseVersion = (Bun.env.RELEASE_VERSION || '').replace(/^v/, '')
  const isNightly = releaseVersion.toLowerCase() === 'nightly' || Bun.env.IS_NIGHTLY === 'true'

  const cargoToml = NodeFS.readFileSync(
    NodePath.join(import.meta.dirname, '..', '..', 'Cargo.toml'),
    'utf-8'
  )

  const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)
  if (!versionMatch) throw new Error('Version not found in Cargo.toml')

  const [, base] = versionMatch
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

function isValidSemver(v: string) {
  return !!v && Bun.semver.satisfies(v, v)
}

async function updateOptionalDependencies(packagePath: string, version: string) {
  const packageJsonPath = NodePath.join(packagePath, 'package.json')
  const packageJson = JSON.parse(NodeFS.readFileSync(packageJsonPath, 'utf-8'))

  if (packageJson.optionalDependencies) {
    Object.keys(packageJson.optionalDependencies).forEach(key => {
      packageJson.optionalDependencies[key] = version
    })

    await Bun.write(packageJsonPath, JSON.stringify(packageJson, null, 2))
  }
}

async function isMetaPackage(packagePath: string) {
  try {
    const packageJsonPath = NodePath.join(packagePath, 'package.json')
    const packageJson = JSON.parse(NodeFS.readFileSync(packageJsonPath, 'utf-8'))
    return packageJson?.name === '@foundry-rs/forge'
  } catch {
    return false
  }
}

async function setPackageVersion(packagePath: string, version: string) {
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

async function packPackage(packagePath: string) {
  let packedFile = ''

  for await (const line of Bun.$`bun pm pack`.cwd(packagePath).lines())
    if (line.endsWith('.tgz')) packedFile = line

  if (!packedFile) throw new Error('Failed to pack package')
  return packedFile
}

async function publishPackage(packagePath: string, packedFile: string, version: string) {
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
