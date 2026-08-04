import { resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import { writeError, writeOutput } from '../../shared/cli-output.mjs'
import {
  formatBuddyRuntimeAssetManifestOutput,
  runBuddyRuntimeAssetManifestCheck,
} from './runtime-asset-manifest.mjs'

export function runBuddyRuntimeAssetManifestCli(options = {}) {
  const result = runBuddyRuntimeAssetManifestCheck(options)
  writeOutput(formatBuddyRuntimeAssetManifestOutput(result))
  return result
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runBuddyRuntimeAssetManifestCli()
  }
  catch (error) {
    writeError(error.message)
    process.exit(1)
  }
}
