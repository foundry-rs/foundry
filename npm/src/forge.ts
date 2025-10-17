import * as NodeChildProcess from 'node:child_process'
import * as NodeFS from 'node:fs'
import * as NodeModule from 'node:module'
import * as NodePath from 'node:path'
import { fileURLToPath } from 'node:url'
import { BINARY_NAME, colors, PLATFORM_SPECIFIC_PACKAGE_NAME } from './const.js'

const require = NodeModule.createRequire(import.meta.url)
const __dirname = NodePath.dirname(fileURLToPath(import.meta.url))

function getBinaryPath() {
  try {
    const binaryPath = require.resolve(`${PLATFORM_SPECIFIC_PACKAGE_NAME}/bin/${BINARY_NAME}`)
    if (NodeFS.existsSync(binaryPath)) return binaryPath
  } catch {
    // Fall back to the binary written by postinstall into dist/
    return NodePath.join(__dirname, '..', 'dist', BINARY_NAME)
  }

  console.error(colors.red, `Platform-specific package ${PLATFORM_SPECIFIC_PACKAGE_NAME} not found.`)
  console.error(colors.yellow, 'This usually means the installation failed or your platform is not supported.')
  console.error(colors.reset)
  console.error(colors.yellow, `Platform: ${process.platform}, Architecture: ${process.arch}`)
  console.error(colors.reset)
  process.exit(1)
}

NodeChildProcess.spawn(getBinaryPath(), process.argv.slice(2), {
  stdio: 'inherit'
})
