import * as NodeChildProcess from 'node:child_process'
import * as NodeFS from 'node:fs'
import * as NodeModule from 'node:module'
import * as Process from 'node:process'

const require = NodeModule.createRequire(import.meta.url)

function getBinaryPath() {
  const { platform, arch } = Process

  let packageName: string | undefined
  let binaryName = 'forge'

  switch (platform) {
    case 'win32':
      binaryName += '.exe'
      if (arch === 'x64') packageName = '@foundry-rs/forge-win32-x64'
      break
    case 'darwin':
      if (arch === 'x64') packageName = '@foundry-rs/forge-darwin-x64'
      else if (arch === 'arm64') packageName = '@foundry-rs/forge-darwin-arm64'
      break
    case 'linux':
      if (arch === 'x64') packageName = '@foundry-rs/forge-linux-x64'
      else if (arch === 'arm64') packageName = '@foundry-rs/forge-linux-arm64'
      break
    default:
      throw new Error(`Unsupported platform: ${platform}-${arch}`)
  }

  if (!packageName) {
    console.error(`Unsupported platform: ${platform}-${arch}`)
    Process.exit(1)
  }

  // Try to find the binary in the platform-specific package
  try {
    const binaryPath = require.resolve(`${packageName}/bin/${binaryName}`)
    if (NodeFS.existsSync(binaryPath)) return binaryPath
  } catch (error) {
    // Package not installed
    console.error(
      'Package not installed',
      error instanceof Error ? error.message : error
    )
  }

  console.error(`Platform-specific package ${packageName} not found.`)
  console.error(
    'This usually means the installation failed or your platform is not supported.'
  )
  console.error(`Platform: ${platform}, Architecture: ${arch}`)
  Process.exit(1)
}

function main() {
  const binaryPath = getBinaryPath()

  // Spawn the binary with all arguments
  const child = NodeChildProcess.spawn(binaryPath, Process.argv.slice(2), {
    stdio: 'inherit',
    windowsHide: false
  })

  child.on('close', (code) => {
    Process.exit(code)
  })

  child.on('error', (error) => {
    console.error('Error executing forge:', error.message)
    Process.exit(1)
  })
}

if (import.meta.url === `file://${Process.argv[1]}`) main()
