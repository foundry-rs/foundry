import { BINARY_NAME, colors, getRegistryUrl, PLATFORM_SPECIFIC_PACKAGE_NAME } from '#const.ts'
import * as NodeCrypto from 'node:crypto'
import * as NodeFS from 'node:fs'
import * as NodeHttp from 'node:http'
import * as NodeHttps from 'node:https'
import * as NodeModule from 'node:module'
import * as NodePath from 'node:path'
import { fileURLToPath } from 'node:url'
import * as NodeZlib from 'node:zlib'

const __dirname = NodePath.dirname(fileURLToPath(import.meta.url))
const fallbackBinaryPath = NodePath.join(__dirname, BINARY_NAME)

const require = NodeModule.createRequire(import.meta.url)

// Accept typical localhost variants by default
const isLocalhostHost = (hostname: string) => (
  hostname === 'localhost'
  || hostname === '127.0.0.1'
  || hostname === '::1'
)

// Enforce HTTPS except for localhost, unless explicitly allowed
function ensureSecureUrl(urlString: string, purpose: string) {
  try {
    const url = new URL(urlString)
    if (url.protocol === 'http:') {
      const allowInsecure = process.env.ALLOW_INSECURE_REGISTRY === 'true'
      if (!isLocalhostHost(url.hostname) && !allowInsecure) {
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

function makeRequest(url: string): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    ensureSecureUrl(url, 'HTTP request')

    const client = url.startsWith('https:') ? NodeHttps : NodeHttp
    client
      .get(url, response => {
        if (response?.statusCode && response.statusCode >= 200 && response.statusCode < 300) {
          const chunks: Array<Buffer> = []

          response.on('data', chunk => chunks.push(chunk))
          response.on('end', () => resolve(Buffer.concat(chunks)))
        } else if (
          response?.statusCode
          && response.statusCode >= 300
          && response.statusCode < 400
          && response.headers.location
        ) {
          // Follow redirects
          const redirected = (() => {
            try {
              return new URL(response.headers.location, url).href
            } catch {
              return response.headers.location
            }
          })()
          makeRequest(redirected).then(resolve, reject)
        } else {
          reject(
            new Error(
              `Package registry responded with status code ${response.statusCode} when downloading the package.`
            )
          )
        }
      })
      .on('error', error => reject(error))
  })
}

/**
 * Scoped package names should be percent-encoded
 * e.g. @scope/pkg -> %40scope%2Fpkg
 */
const encodePackageNameForRegistry = (name: string) => name.startsWith('@') ? encodeURIComponent(name) : name

/**
 * Tar archives are organized in 512 byte blocks.
 * Blocks can either be header blocks or data blocks.
 * Header blocks contain file names of the archive in the first 100 bytes, terminated by a null byte.
 * The size of a file is contained in bytes 124-135 of a header block and in octal format.
 * The following blocks will be data blocks containing the file.
 */
function extractFileFromTarball(
  tarballBuffer: Buffer<ArrayBufferLike>,
  filepath: string
): Buffer<ArrayBufferLike> {
  let offset = 0
  while (offset < tarballBuffer.length) {
    const header = tarballBuffer.subarray(offset, offset + 512)
    offset += 512

    const fileName = header.toString('utf-8', 0, 100).replace(/\0.*/g, '')
    const fileSize = Number.parseInt(header.toString('utf-8', 124, 136).replace(/\0.*/g, ''), 8)

    if (fileName === filepath)
      return tarballBuffer.subarray(offset, offset + fileSize)

    // Clamp offset to the uppoer multiple of 512
    offset = (offset + fileSize + 511) & ~511
  }
  throw new Error(`File ${filepath} not found in tarball`)
}

async function downloadBinaryFromRegistry() {
  const registryUrl = getRegistryUrl().replace(/\/$/, '')
  ensureSecureUrl(registryUrl, 'registry URL')

  const encodedName = encodePackageNameForRegistry(PLATFORM_SPECIFIC_PACKAGE_NAME)

  // Determine which version to fetch: prefer the version pinned in optionalDependencies
  let desiredVersion: string | undefined
  try {
    const pkgJsonPath = NodePath.join(__dirname, '..', 'package.json')
    const pkgJson = JSON.parse(NodeFS.readFileSync(pkgJsonPath, 'utf8'))
    desiredVersion = pkgJson?.optionalDependencies?.[PLATFORM_SPECIFIC_PACKAGE_NAME] || pkgJson?.version
  } catch {}

  // Fetch metadata for the platform-specific package
  const metaUrl = `${registryUrl}/${encodedName}`
  const metaBuffer = await makeRequest(metaUrl)
  const metadata = JSON.parse(metaBuffer.toString('utf8'))

  const version = desiredVersion || metadata?.['dist-tags']?.latest
  const versionMeta = metadata?.versions?.[version]
  const dist = versionMeta?.dist
  if (!dist?.tarball) {
    throw new Error(
      `Could not find tarball for ${PLATFORM_SPECIFIC_PACKAGE_NAME}@${version} from ${metaUrl}`
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
  const tarballDownloadBuffer = await makeRequest(dist.tarball)
  ;(() => {
    let verified = false

    const integrity = typeof dist.integrity === 'string' ? dist.integrity : ''
    const sriMatch = integrity.match(/^([a-z0-9]+)-([A-Za-z0-9+/=]+)$/i)
    if (sriMatch) {
      const algo = sriMatch[1].toLowerCase()
      const expected = sriMatch[2]
      const allowed = new Set(['sha512', 'sha256', 'sha1'])
      if (allowed.has(algo)) {
        const actual = NodeCrypto.createHash(algo as 'sha512')
          .update(tarballDownloadBuffer)
          .digest('base64')
        if (expected !== actual)
          throw new Error(`Downloaded tarball failed integrity check (${algo} mismatch)`)
        verified = true
      }
    }

    if (!verified && typeof dist.shasum === 'string' && dist.shasum.length === 40) {
      const expectedSha1Hex = dist.shasum.toLowerCase()
      const actualSha1Hex = NodeCrypto.createHash('sha1')
        .update(tarballDownloadBuffer)
        .digest('hex')
      if (expectedSha1Hex !== actualSha1Hex)
        throw new Error('Downloaded tarball failed integrity check (sha1 shasum mismatch)')
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
  })()

  // Unpack and write binary
  const tarballBuffer = NodeZlib.gunzipSync(tarballDownloadBuffer)

  NodeFS.writeFileSync(
    fallbackBinaryPath,
    extractFileFromTarball(tarballBuffer, `package/bin/${BINARY_NAME}`),
    { mode: 0o755 } // Make binary file executable
  )
}

function isPlatformSpecificPackageInstalled() {
  try {
    // Resolving will fail if the optionalDependency was not installed
    require.resolve(`${PLATFORM_SPECIFIC_PACKAGE_NAME}/bin/${BINARY_NAME}`)
    return true
  } catch (_error) {
    return false
  }
}

if (!PLATFORM_SPECIFIC_PACKAGE_NAME)
  throw new Error('Platform not supported!')

// Skip downloading the binary if it was already installed via optionalDependencies
if (!isPlatformSpecificPackageInstalled()) {
  console.log('Platform specific package not found. Will manually download binary.')
  downloadBinaryFromRegistry()
} else {
  console.log('Platform specific package already installed. Skipping manual download.')
}
