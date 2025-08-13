import NodeChildProcess from 'node:child_process'
import NodeModule from 'node:module'
import NodePath from 'node:path'
import { BINARY_NAME, PLATFORM_SPECIFIC_PACKAGE_NAME } from './const.js'

const require = NodeModule.createRequire(import.meta.url)

function getBinaryPath() {
  try {
    return require.resolve(`${PLATFORM_SPECIFIC_PACKAGE_NAME}/bin/${BINARY_NAME}`)
  } catch (_error) {
    return NodePath.join(
      import.meta.dirname,
      '..',
      '..',
      'bin',
      BINARY_NAME
    )
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  NodeChildProcess.execFileSync(
    getBinaryPath(),
    process.argv.slice(2),
    { stdio: 'inherit' }
  )
}
