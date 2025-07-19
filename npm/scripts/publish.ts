#!/usr/bin/env bun
import { colors } from '#utilities.ts'
import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'

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

  const packagePath = Bun.argv[2]
  if (!packagePath) throw new Error('Package path is required')
  console.info(colors.green, 'Package path:', packagePath)

  const publishVersion = getPublishVersion()
  console.info(colors.green, 'Publish version:', publishVersion)

  if (packagePath === '@foundry-rs/forge')
    await updateOptionalDependencies(packagePath, publishVersion)

  await setPackageVersion(packagePath, publishVersion, npmToken)
  const packedFile = await packPackage(packagePath)
  await publishPackage(packagePath, packedFile)
}

function getPublishVersion() {
  if (Bun.env.VERSION_NAME) return Bun.env.VERSION_NAME.replace(/^v/, '')
  if (Bun.env.BUMP_VERSION) return Bun.env.BUMP_VERSION

  const cargoToml = NodeFS.readFileSync(
    NodePath.join(import.meta.dirname, '..', '..', 'Cargo.toml'),
    'utf-8'
  )

  const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)
  if (!versionMatch) throw new Error('Version not found in Cargo.toml')

  return versionMatch[1]
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

async function setPackageVersion(packagePath: string, version: string, npmToken: string) {
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

async function publishPackage(packagePath: string, packedFile: string) {
  const result = await Bun.$`npm publish ./${packedFile} --access=public --registry=${REGISTRY_URL}`
    .cwd(packagePath)
    .nothrow()

  if (result.exitCode !== 0)
    throw new Error(`Publish failed: ${result.stderr}`)
}
