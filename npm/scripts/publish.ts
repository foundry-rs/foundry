#!/usr/bin/env bun
import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'
import * as Bun from 'bun'
import { colors } from '../src/utilities.ts'

/**
 * TODO:
 * - handle publishing `@foundry-rs/forge`
 *    - auto-bump own version,
 *    - auto-bump versions in `optionalDependencies`,
 */

main().catch((error) => {
  console.error(error)
  process.exit(1)
})

async function main() {
  const packagePath = Bun.argv.at(2)
  if (!packagePath) throw new Error('Package path is required')

  const publishVersion = (() => {
    if (Bun.env.BUMP_VERSION) return Bun.env.BUMP_VERSION
    const cargoToml = NodeFS.readFileSync(
      NodePath.join(import.meta.dirname, '..', '..', 'Cargo.toml'),
      'utf-8'
    )
    for (const line of cargoToml.split('\n')) {
      if (!line.toLowerCase().startsWith('version = "')) continue
      const [, publishVersion] = line.split('"')
      return publishVersion
    }
    throw new Error('Version not found in Cargo.toml')
  })()

  const NPM_TOKEN = Bun.env.NPM_TOKEN
  if (!NPM_TOKEN) throw new Error('NPM_TOKEN is required')

  console.info(colors.green, 'Publish version:', publishVersion)

  const bumpVersion = await Bun.$`
    npm version ${publishVersion} \
      --message "TODO: Add message" \
      --git-tag-version=false \
      --workspace-update=false \
      --sign-git-tag=false \
      --git-tag-version=false \
      --commit-hook=false \
      --allow-same-version`
    .cwd(packagePath)
    .env({
      ...Bun.env,
      ...process.env,
      NPM_TOKEN
    })
    .quiet()
    .nothrow()

  if (bumpVersion.exitCode !== 0) throw new Error(bumpVersion.stderr.toString())

  console.log(bumpVersion.stdout.toString())

  let packedFile: string | undefined

  for await (const line of Bun.$`bun pm pack`.cwd(packagePath).lines()) {
    console.info(line)
    if (line.endsWith('.tgz')) packedFile = line
  }
  console.info(colors.green, 'Packed file:', packedFile)

  const publishPackage =
    await Bun.$`bun publish --access='public' --verbose --registry='https://registry.npmjs.org' ./${packedFile}`
      .cwd(packagePath)
      .quiet()
      .nothrow()
  console.info(publishPackage.stdout.toString())
  console.info(publishPackage.stderr.toString())
  if (publishPackage.exitCode !== 0)
    throw new Error(publishPackage.stderr.toString())

  console.log(publishPackage.stdout.toString())
}
