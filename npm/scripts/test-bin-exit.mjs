#!/usr/bin/env node

import * as NodeAssert from 'node:assert/strict'
import * as NodeChildProcess from 'node:child_process'
import * as NodeFS from 'node:fs/promises'
import * as NodeOS from 'node:os'
import * as NodePath from 'node:path'
import { fileURLToPath } from 'node:url'

import { BINARY_NAME, PLATFORM_SPECIFIC_PACKAGE_NAME } from '#const.mjs'

const __dirname = NodePath.dirname(fileURLToPath(import.meta.url))
const tool = 'forge'
const binaryName = BINARY_NAME(tool)
const maybePlatformPackage = PLATFORM_SPECIFIC_PACKAGE_NAME(tool)

if (!maybePlatformPackage)
  throw new Error(`Unsupported platform for ${tool}: ${process.platform}/${process.arch}`)

const platformPackage = maybePlatformPackage
const tempDir = await NodeFS.mkdtemp(NodePath.join(NodeOS.tmpdir(), 'foundry-npm-bin-'))

try {
  await stageFakeBinary()
  assertExitCode(0)
  assertExitCode(7)
} finally {
  await NodeFS.rm(tempDir, { recursive: true, force: true })
}

async function stageFakeBinary() {
  const packageDir = NodePath.join(tempDir, 'node_modules', ...platformPackage.split('/'))
  const binDir = NodePath.join(packageDir, 'bin')
  const binaryPath = NodePath.join(binDir, binaryName)

  await NodeFS.mkdir(binDir, { recursive: true })
  await NodeFS.writeFile(
    NodePath.join(packageDir, 'package.json'),
    JSON.stringify({ name: platformPackage, version: '0.0.0-test' }, null, 2) + '\n'
  )
  await NodeFS.writeFile(
    binaryPath,
    [
      '#!/usr/bin/env node',
      'process.exit(Number(process.argv[2]))',
      ''
    ].join('\n'),
    { mode: 0o755 }
  )
  await NodeFS.chmod(binaryPath, 0o755)
}

/**
 * @param {number} expected
 */
function assertExitCode(expected) {
  const result = NodeChildProcess.spawnSync(
    process.execPath,
    ['./src/bin.mjs', String(expected)],
    {
      cwd: NodePath.resolve(__dirname, '..'),
      env: {
        ...process.env,
        NODE_PATH: NodePath.join(tempDir, 'node_modules'),
        TARGET_TOOL: tool
      },
      encoding: 'utf8'
    }
  )

  NodeAssert.equal(result.status, expected, result.stderr || result.stdout)
}
