import * as NodeCrypto from 'node:crypto'
import * as NodeFS from 'node:fs'
import * as NodeHttp from 'node:http'
import * as NodeHttps from 'node:https'
import * as NodeModule from 'node:module'
import * as NodePath from 'node:path'
import { fileURLToPath } from 'node:url'
import * as NodeZlib from 'node:zlib'
import { BINARY_NAME, colors, getRegistryUrl, PLATFORM_SPECIFIC_PACKAGE_NAME } from './const.js'

const __dirname = NodePath.dirname(fileURLToPath(import.meta.url))
const fallbackBinaryPath = NodePath.join(__dirname, BINARY_NAME)

const require = NodeModule.createRequire(import.meta.url)

function makeRequest(url: string): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const client = url.startsWith('https:') ? NodeHttps : NodeHttp
    client
      .get(url, response => {
        if (response?.statusCode && response.statusCode >= 200 && response.statusCode < 300) {
          const chunks: Buffer[] = []
          response.on('data', chunk => chunks.push(chunk))
          response.on('end', () => {
            resolve(Buffer.concat(chunks))
          })
        } else if (
          response?.statusCode
          && response.statusCode >= 300
          && response.statusCode < 400
          && response.headers.location
        ) {
          // Follow redirects
          makeRequest(response.headers.location).then(resolve, reject)
        } else {
          reject(
            new Error(
              `npm responded with status code ${response.statusCode} when downloading the package!`
            )
          )
        }
      })
      .on('error', error => {
        reject(error)
      })
  })
}

function encodePackageNameForRegistry(name: string) {
  // Scoped package names should be percent-encoded
  // e.g. @scope/pkg -> %40scope%2Fpkg
  return name.startsWith('@') ? encodeURIComponent(name) : name
}

function extractFileFromTarball(
  tarballBuffer: Buffer<ArrayBufferLike>,
  filepath: string
): Buffer<ArrayBufferLike> {
  // Tar archives are organized in 512 byte blocks.
  // Blocks can either be header blocks or data blocks.
  // Header blocks contain file names of the archive in the first 100 bytes, terminated by a null byte.
  // The size of a file is contained in bytes 124-135 of a header block and in octal format.
  // The following blocks will be data blocks containing the file.
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

async function downloadBinaryFromNpm() {
  const registryUrl = getRegistryUrl().replace(/\/$/, '')
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

  console.info(
    colors.green,
    'Downloading binary from:\n',
    dist.tarball,
    '\n',
    colors.reset
  )

  // Download the tarball of the right binary distribution package
  const tarballDownloadBuffer = await makeRequest(dist.tarball)

  // Verify integrity if available
  if (dist.integrity && dist.integrity.startsWith('sha512-')) {
    const expected = dist.integrity.split('sha512-')[1]
    const actual = NodeCrypto.createHash('sha512')
      .update(tarballDownloadBuffer)
      .digest('base64')
    if (expected !== actual)
      throw new Error('Downloaded tarball failed integrity check (sha512 mismatch)')
  }

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
  downloadBinaryFromNpm()
} else {
  console.log('Platform specific package already installed. Skipping manual download.')
}
