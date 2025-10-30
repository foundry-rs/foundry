#!/usr/bin/env node

import * as NodeCrypto from 'node:crypto'
import * as NodeFS from 'node:fs'
import * as NodeHttp from 'node:http'
import * as NodeHttps from 'node:https'
import * as NodeModule from 'node:module'
import * as NodePath from 'node:path'
import { fileURLToPath } from 'node:url'
import * as NodeZlib from 'node:zlib'

import { BINARY_NAME, colors, getRegistryUrl, PLATFORM_SPECIFIC_PACKAGE_NAME, resolveTargetTool } from './const.mjs'

const __dirname = NodePath.dirname(fileURLToPath(import.meta.url))
const targetTool = resolveTargetTool()
const binaryName = BINARY_NAME(targetTool)
const fallbackBinaryPath = NodePath.join(__dirname, binaryName)
const platformSpecificPackageName = PLATFORM_SPECIFIC_PACKAGE_NAME(targetTool)

const expectedTarEntryPath = `package/bin/${binaryName}`

if (NodePath.relative(__dirname, fallbackBinaryPath).startsWith('..'))
  throw new Error('Resolved binary path escapes package directory')

if (!platformSpecificPackageName) throw new Error('Platform not supported!')

const require = NodeModule.createRequire(import.meta.url)

/**
 * Enforce HTTPS except for localhost, unless explicitly allowed
 * @param {string} urlString
 * @param {string} purpose
 * @returns {void}
 */
function ensureSecureUrl(urlString, purpose) {
  try {
    const url = new URL(urlString)
    if (url.protocol === 'http:') {
      const allowInsecure = process.env.ALLOW_INSECURE_REGISTRY === 'true'
      if (
        // Accept typical localhost variants by default
        !['localhost', '127.0.0.1', '::1'].includes(url.hostname)
        && !allowInsecure
      ) {
        throw new Error(
          `Refusing to use insecure HTTP for ${purpose}: ${urlString}. `
            + `Set ALLOW_INSECURE_REGISTRY=true to override (not recommended).`
        )
      }
    }
  } catch {
    // If parsing fails, the request will fail so no need to do anything here
  }
}

const MAX_REDIRECTS = 10
const REQUEST_TIMEOUT = 30_000 // 30s
const MAX_METADATA_BYTES = 5 * 1024 * 1024 // 5 MiB
const MAX_TARBALL_BYTES = 200 * 1024 * 1024 // 200 MiB
const MAX_BINARY_BYTES = 500 * 1024 * 1024 // 500 MiB

/**
 * @param {string} url
 * @param {{
 *  parentSignal?: AbortSignal
 *  redirectDepth?: number
 *  visited?: Set<string>
 *  maxBytes?: number
 *  collect?: boolean
 *  onChunk?: (chunk: Buffer, response: import('node:http').IncomingMessage) => void
 * } | undefined} options
 * @returns {Promise<Buffer | undefined>}
 */
export function makeRequest(url, options = {}) {
  const {
    parentSignal,
    redirectDepth = 0,
    visited = new Set(),
    maxBytes,
    collect = true,
    onChunk
  } = options

  if (redirectDepth > MAX_REDIRECTS) throw new Error('Maximum redirect depth exceeded')

  if (visited.has(url)) throw new Error('Circular redirect detected')
  visited.add(url)

  ensureSecureUrl(url, 'HTTP request')

  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT)
  const signal = parentSignal
    ? AbortSignal.any([parentSignal, controller.signal])
    : controller.signal

  return new Promise((resolve, reject) => {
    const client = url.startsWith('https:') ? NodeHttps : NodeHttp
    const request = client.get(url, { signal }, response => {
      /**
       * @param {Error | null} error
       * @param {Buffer<ArrayBufferLike> | undefined} value
       */
      const finish = (error, value = undefined) => {
        clearTimeout(timer)
        if (error) reject(error)
        else resolve(value)
      }

      if (
        response?.statusCode
        && response.statusCode >= 200
        && response.statusCode < 300
      ) {
        let totalBytes = 0
        /** @type {Array<Buffer> | undefined} */
        const chunks = collect ? [] : undefined

        response.on('data', chunk => {
          totalBytes += chunk.length
          if (maxBytes && totalBytes > maxBytes) {
            response.destroy(new Error('Response exceeded maximum allowed size'))
            return
          }

          if (chunks) chunks.push(chunk)

          if (onChunk) {
            try {
              onChunk(chunk, response)
            } catch (error) {
              response.destroy(
                error instanceof Error ? error : new Error(`Error occurred: ${error}`)
              )
            }
          }
        })

        response.on('end', () => {
          finish(null, chunks ? Buffer.concat(chunks) : undefined)
        })

        response.on('error', finish)
      } else if (
        response?.statusCode
        && response.statusCode >= 300
        && response.statusCode < 400
        && response.headers.location
      ) {
        clearTimeout(timer)
        const nextUrl = new URL(response.headers.location, url).href

        return makeRequest(nextUrl, {
          parentSignal: signal,
          redirectDepth: redirectDepth + 1,
          visited,
          maxBytes,
          collect,
          onChunk
        }).then(resolve, reject)
      } else {
        finish(
          new Error(
            `Package registry responded with status code ${
              response?.statusCode ?? '(none)'
            } when downloading the package.`
          )
        )
      }
    })

    request.on('error', error => {
      clearTimeout(timer)
      reject(error)
    })
  })
}

/**
 * Tar archives are organized in 512 byte blocks.
 * Blocks can either be header blocks or data blocks.
 * Header blocks contain file names of the archive in the first 100 bytes, terminated by a null byte.
 * The size of a file is contained in bytes 124-135 of a header block and in octal format.
 * The following blocks will be data blocks containing the file.
 * @param {Buffer<ArrayBufferLike>} tarballBuffer
 * @param {string} filepath
 * @returns {Buffer<ArrayBufferLike>}
 */
function extractFileFromTarball(tarballBuffer, filepath) {
  let offset = 0
  while (offset < tarballBuffer.length) {
    const header = tarballBuffer.subarray(offset, offset + 512)
    offset += 512

    const fileName = header.toString('utf-8', 0, 100).replace(/\0.*/g, '')
    const fileSize = Number.parseInt(
      header.toString('utf-8', 124, 136).replace(/\0.*/g, ''),
      8
    )

    if (fileName === filepath) {
      if (!Number.isFinite(fileSize) || Number.isNaN(fileSize) || fileSize < 0)
        throw new Error(`Invalid size for ${filepath} in tarball`)
      if (fileSize > MAX_BINARY_BYTES)
        throw new Error(`Binary size for ${filepath} exceeds maximum allowed threshold`)
      return tarballBuffer.subarray(offset, offset + fileSize)
    }

    // Clamp offset to the upper multiple of 512
    offset = (offset + fileSize + 511) & ~511
  }
  throw new Error(`File ${filepath} not found in tarball`)
}

async function downloadBinaryFromRegistry() {
  if (!platformSpecificPackageName)
    throw new Error('Platform-specific package name is not defined')

  const registryUrl = getRegistryUrl().replace(/\/$/, '')
  ensureSecureUrl(registryUrl, 'registry URL')

  // Scoped package names should be percent-encoded
  const encodedName = platformSpecificPackageName.startsWith('@')
    ? encodeURIComponent(platformSpecificPackageName)
    : platformSpecificPackageName

  // Determine which version to fetch: prefer the version pinned in optionalDependencies
  /** @type {string | undefined} */
  let desiredVersion
  try {
    const pkgJsonPath = NodePath.join(__dirname, '..', 'package.json')
    const pkgJson = JSON.parse(NodeFS.readFileSync(pkgJsonPath, 'utf8'))
    desiredVersion = pkgJson?.optionalDependencies[platformSpecificPackageName]
      || pkgJson?.version
  } catch {}

  // Fetch metadata for the platform-specific package
  const metaUrl = `${registryUrl}/${encodedName}`
  const metaBuffer = await makeRequest(metaUrl, { maxBytes: MAX_METADATA_BYTES })
  if (!metaBuffer)
    throw new Error('Failed to download package metadata')
  const metadata = JSON.parse(metaBuffer.toString('utf8'))

  const version = desiredVersion || metadata?.['dist-tags']?.latest
  const versionMeta = metadata?.versions?.[version]
  const dist = versionMeta?.dist
  if (!dist?.tarball) {
    throw new Error(
      `Could not find tarball for ${platformSpecificPackageName}@${version} from ${metaUrl}`
    )
  }

  // Guard tarball URL scheme
  ensureSecureUrl(dist.tarball, 'tarball URL')

  console.info(
    colors.green,
    'Downloading binary from:\n',
    dist.tarball,
    '\n',
    colors.reset
  )

  /**
   * Download the tarball of the right binary distribution package
   * Verify integrity: prefer SRI integrity (sha512/sha256/sha1),
   * fallback to legacy dist.shasum (sha1 hex). Fail if neither unless explicitly allowed.
   */
  const integrity = typeof dist.integrity === 'string' ? dist.integrity : ''
  const sriMatch = integrity.match(/^([a-z0-9]+)-([A-Za-z0-9+/=]+)$/i)
  const allowedSRIAlgorithms = new Set(['sha512', 'sha256', 'sha1'])
  const sriAlgo = sriMatch && allowedSRIAlgorithms.has(sriMatch[1].toLowerCase())
    ? sriMatch[1].toLowerCase()
    : undefined
  const expectedSri = sriAlgo ? sriMatch?.[2] : undefined
  const sriHasher = sriAlgo ? NodeCrypto.createHash(sriAlgo) : undefined

  const expectedSha1Hex = typeof dist.shasum === 'string' && dist.shasum.length === 40
    ? dist.shasum.toLowerCase()
    : undefined
  const sha1Hasher = expectedSha1Hex ? NodeCrypto.createHash('sha1') : undefined

  const tarballDownloadBuffer = await makeRequest(dist.tarball, {
    maxBytes: MAX_TARBALL_BYTES,
    onChunk: chunk => {
      sriHasher?.update(chunk)
      sha1Hasher?.update(chunk)
    }
  })

  if (!tarballDownloadBuffer)
    throw new Error('Failed to download tarball contents')

  let verified = false

  if (sriHasher && expectedSri) {
    const actual = sriHasher.digest('base64')
    if (expectedSri !== actual) {
      throw new Error(
        `Downloaded tarball failed integrity check (${sriAlgo} mismatch)`
      )
    }
    verified = true
  }

  if (!verified && sha1Hasher && expectedSha1Hex) {
    const actualSha1Hex = sha1Hasher.digest('hex')
    if (expectedSha1Hex !== actualSha1Hex) {
      throw new Error(
        'Downloaded tarball failed integrity check (sha1 shasum mismatch)'
      )
    }
    verified = true
  }

  if (!verified) {
    const allowNoIntegrity = process.env.ALLOW_NO_INTEGRITY === 'true'
      || process.env.ALLOW_UNVERIFIED_TARBALL === 'true'
    if (!allowNoIntegrity) {
      throw new Error(
        'No integrity metadata found for downloaded tarball. '
          + 'Set ALLOW_NO_INTEGRITY=true to bypass (not recommended).'
      )
    }
    console.warn(
      colors.yellow,
      'Warning: proceeding without integrity verification (explicitly allowed).',
      colors.reset
    )
  }

  // Unpack and write binary
  const tarballBuffer = NodeZlib.gunzipSync(tarballDownloadBuffer)

  NodeFS.writeFileSync(
    fallbackBinaryPath,
    extractFileFromTarball(tarballBuffer, expectedTarEntryPath),
    { mode: 0o755 } // Make binary file executable
  )
}

function isPlatformSpecificPackageInstalled() {
  try {
    // Resolving will fail if the optionalDependency was not installed
    require.resolve(`${platformSpecificPackageName}/bin/${binaryName}`)
    return true
  } catch {
    return false
  }
}

// Skip downloading the binary if it was already installed via optionalDependencies
if (!isPlatformSpecificPackageInstalled()) {
  console.log('Platform specific package not found. Will manually download binary.')
  downloadBinaryFromRegistry().catch(error => {
    console.error(colors.red, 'Failed to download binary:', error, colors.reset)
    process.exitCode = 1
  })
} else {
  console.log(
    'Platform specific package already installed. Skipping manual download.'
  )
}
