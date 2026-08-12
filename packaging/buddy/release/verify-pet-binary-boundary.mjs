import { Buffer } from 'node:buffer'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import process from 'node:process'

import { writeOutput } from '../../shared/cli-output.mjs'

const repoRoot = resolve(import.meta.dirname, '../../..')
const defaultRuntimePath = resolve(
  repoRoot,
  'apps/buddy/runtime/target/release/lexora-buddy-runtime',
)
const defaultPetPath = resolve(
  repoRoot,
  'apps/buddy/runtime/target/release/lexora-buddy-pet',
)
const runtimeOnlyMarkers = [
  'chat.startTurn',
  'codex.listModels',
  'codex runtime failed',
  'runtime.shutdown',
  'sqlite operation failed',
]

export function verifyPetBinaryBoundary(options = {}) {
  const runtimePath = options.runtimePath ?? defaultRuntimePath
  const petPath = options.petPath ?? defaultPetPath
  const runtime = readFileSync(runtimePath)
  const pet = readFileSync(petPath)
  const errors = []

  if (pet.equals(runtime))
    errors.push('standalone pet must not reuse the Desktop Runtime executable')

  for (const marker of runtimeOnlyMarkers) {
    if (pet.includes(Buffer.from(marker)))
      errors.push(`standalone pet contains Desktop Runtime protocol marker: ${marker}`)
  }

  return errors
}

export function assertPetBinaryBoundary(options = {}) {
  const errors = verifyPetBinaryBoundary(options)
  if (errors.length)
    throw new Error(errors.join('\n'))
}

if (process.argv[1] === new URL(import.meta.url).pathname) {
  assertPetBinaryBoundary()
  writeOutput('Standalone pet binary boundary check passed')
}
