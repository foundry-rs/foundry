import { BINARY_NAME, colors, KNOWN_TOOLS, PLATFORM_SPECIFIC_PACKAGE_NAME, resolveTargetTool } from '#const.mjs'
import * as NodeChildProcess from 'node:child_process'
import * as NodeFS from 'node:fs'
import * as NodeModule from 'node:module'
import * as NodePath from 'node:path'
import { fileURLToPath } from 'node:url'

const require = NodeModule.createRequire(import.meta.url)
const __dirname = NodePath.dirname(fileURLToPath(import.meta.url))

const targetTool = resolveTool()
process.env.TARGET_TOOL ??= targetTool

const binaryName = BINARY_NAME(targetTool)
const platformPackage = PLATFORM_SPECIFIC_PACKAGE_NAME(targetTool)

if (!platformPackage) {
  console.error(colors.red, 'Platform not supported!')
  console.error(colors.reset)
  console.error(colors.yellow, `Platform: ${process.platform}, Architecture: ${process.arch}`)
  console.error(colors.reset)
  process.exit(1)
}

NodeChildProcess.spawn(selectBinaryPath(), process.argv.slice(2), { stdio: 'inherit' })

function resolveTool() {
  const candidates = [
    process.env.TARGET_TOOL,
    toolFromPackageName(process.env.npm_package_name),
    toolFromLocalPackage(),
    toolFromPath()
  ]

  for (const candidate of candidates) {
    if (!candidate) continue
    try {
      return resolveTargetTool(candidate)
    } catch {
      // try next
    }
  }

  throw new Error('TARGET_TOOL must be set to one of: ' + KNOWN_TOOLS.join(', '))
}

function toolFromLocalPackage() {
  try {
    const packageJsonPath = NodePath.join(__dirname, 'package.json')
    if (!NodeFS.existsSync(packageJsonPath)) return undefined
    const pkg = require(packageJsonPath)
    return toolFromPackageName(pkg?.name)
  } catch {
    return undefined
  }
}

function toolFromPackageName(name) {
  if (typeof name !== 'string') return undefined
  const match = name.match(/^@foundry-rs\/(forge|cast|anvil|chisel)(?:$|-)/)
  return match ? match[1] : undefined
}

function toolFromPath() {
  const segments = NodePath.resolve(__dirname).split(NodePath.sep)
  for (let i = segments.length - 1; i >= 0; i--) {
    const candidate = segments[i]
    if (KNOWN_TOOLS.includes(candidate)) return candidate
  }
  return undefined
}

function selectBinaryPath() {
  try {
    const candidate = require.resolve(`${platformPackage}/bin/${binaryName}`)
    if (NodeFS.existsSync(candidate)) return candidate
  } catch {
    // fall through to dist/ binary
  }

  return NodePath.join(__dirname, '..', 'dist', binaryName)
}
