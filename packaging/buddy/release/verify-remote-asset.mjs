import { Buffer } from 'node:buffer'
import { createHash } from 'node:crypto'
import { get as httpsGet } from 'node:https'
import { resolve } from 'node:path'
import process from 'node:process'

import { writeError, writeOutput } from '../../shared/cli-output.mjs'
import { readBuddyDebReleaseMetadata } from '../ci/verify-linux-deb-artifact.mjs'

const DEFAULT_DOWNLOAD_TIMEOUT_MS = 30_000
const DEFAULT_MAX_ASSET_BYTES = 1_000_000_000
const DEFAULT_MAX_REDIRECTS = 5

export async function verifyLexoraBuddyRemoteAsset(metadata, downloadAsset = downloadHttpsAsset) {
  const content = await downloadAsset(metadata.sourceUrl)
  const hash = createHash('sha256').update(content).digest('hex')
  if (hash !== metadata.expectedHash)
    throw new Error('remote asset hash does not match PKGBUILD')

  return {
    byteLength: content.byteLength,
    hash,
    releaseAssetName: metadata.releaseAssetName,
    sourceUrl: metadata.sourceUrl,
  }
}

export function downloadHttpsAsset(sourceUrl, options = {}) {
  const timeoutMs = options.timeoutMs ?? DEFAULT_DOWNLOAD_TIMEOUT_MS
  const maxAssetBytes = options.maxAssetBytes ?? DEFAULT_MAX_ASSET_BYTES
  const maxRedirects = options.maxRedirects ?? DEFAULT_MAX_REDIRECTS
  const getImpl = options.getImpl ?? httpsGet

  return download(sourceUrl, maxRedirects)

  function download(url, redirectsRemaining) {
    return new Promise((resolveDownload, rejectDownload) => {
      const request = getImpl(url, (response) => {
        const statusCode = response.statusCode ?? 0
        const location = response.headers?.location
        if (statusCode >= 300 && statusCode < 400 && location) {
          response.resume()
          if (redirectsRemaining <= 0) {
            rejectDownload(new Error('remote asset download exceeded the redirect limit'))
            return
          }
          const redirectedUrl = new URL(location, url)
          if (redirectedUrl.protocol !== 'https:') {
            rejectDownload(new Error('remote asset redirect must use https'))
            return
          }
          download(redirectedUrl.href, redirectsRemaining - 1).then(resolveDownload, rejectDownload)
          return
        }
        if (statusCode !== 200) {
          response.resume()
          rejectDownload(new Error(`remote asset download returned HTTP ${statusCode}`))
          return
        }

        const chunks = []
        let byteLength = 0
        response.on('data', (chunk) => {
          byteLength += chunk.length
          if (byteLength > maxAssetBytes) {
            request.destroy(new Error(`remote asset exceeds ${maxAssetBytes} bytes`))
            return
          }
          chunks.push(chunk)
        })
        response.once('end', () => resolveDownload(Buffer.concat(chunks)))
      })
      request.setTimeout(timeoutMs, () => {
        request.destroy(new Error(`remote asset download timed out after ${timeoutMs}ms`))
      })
      request.on('error', rejectDownload)
    })
  }
}

async function main() {
  const result = await verifyLexoraBuddyRemoteAsset(readBuddyDebReleaseMetadata())
  writeOutput(`Buddy remote release asset passed: ${result.releaseAssetName} (${result.byteLength} bytes)`)
}

if (process.argv[1] && resolve(process.argv[1]) === new URL(import.meta.url).pathname) {
  void main().catch((error) => {
    writeError(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  })
}
