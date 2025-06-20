import * as NodeFS from 'node:fs'
import * as NodePath from 'node:path'
import { defineConfig, type UserConfig } from 'tsdown'

/**
 * this makes it so that the bin automatically runs if bun, deno, or node is installed
 */
const shebang = /* sh */ `#!/bin/sh
//bin/true; (command -v bun && bun $0) || (command -v deno && deno --allow-all $0) || (command -v node && node $0) || exit 1

`

const config = {
  dts: false,
  clean: true,
  format: ['es'],
  target: 'node20',
  platform: 'node',
  outExtensions: () => ({ js: '.mjs' }),
  onSuccess: ({ name }) => console.info(`🎉 [${name}] Build complete!`),
  hooks: {
    'build:before': ({ options }) => {
      const packagePath = options.env?.PACKAGE_PATH
      if (!packagePath) return

      NodeFS.readdirSync(packagePath, { withFileTypes: true })
        .filter((item) => !['package.json', 'README.md'].includes(item.name))
        .forEach((item) =>
          NodeFS.rmSync(NodePath.join(packagePath, item.name), {
            recursive: true,
            force: true
          })
        )
    },
    'build:done': ({ options }) => {
      // prepend shebang to the file
      const normalizedPath = NodePath.join(
        options.outDir,
        `${options.name}.mjs`
      )
      NodeFS.writeFileSync(
        normalizedPath,
        shebang + NodeFS.readFileSync(normalizedPath, { encoding: 'utf8' }),
        { encoding: 'utf8' }
      )
    }
  }
} as const satisfies UserConfig

export default [
  defineConfig({
    ...config,
    name: 'forge',
    env: {
      PACKAGE_PATH: './@foundry-rs/forge'
    },
    outDir: './@foundry-rs/forge/bin',
    entry: ['./src/forge.ts']
  }),
  defineConfig({
    ...config,
    name: 'install',
    entry: ['./src/install.ts'],
    outDir: './@foundry-rs/forge/dist',
    outExtensions: () => ({ js: '.mjs' })
  })
]
