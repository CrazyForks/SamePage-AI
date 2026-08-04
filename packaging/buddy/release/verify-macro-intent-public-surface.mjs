import { resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import { writeError, writeOutput } from '../../shared/cli-output.mjs'
import {
  formatBuddyMacroIntentPublicSurfaceOutput,
  runBuddyMacroIntentPublicSurfaceCheck,
} from './macro-intent-public-surface.mjs'

export function runBuddyMacroIntentPublicSurfaceCli(options = {}) {
  const result = runBuddyMacroIntentPublicSurfaceCheck(options)
  writeOutput(formatBuddyMacroIntentPublicSurfaceOutput(result))
  return result
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runBuddyMacroIntentPublicSurfaceCli()
  }
  catch (error) {
    writeError(error.message)
    process.exit(1)
  }
}
