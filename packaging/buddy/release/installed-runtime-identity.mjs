import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import { writeError, writeOutput } from '../../shared/cli-output.mjs'
import {
  createLexoraBuddyReleaseMetadata,
  validateLexoraBuddyReleaseMetadata,
} from './metadata.mjs'

const scriptDir = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(scriptDir, '../../..')

export function evaluateBuddyInstalledRuntimeIdentity(input) {
  const capabilities = input.capabilities ?? null
  const metadata = input.metadata
  const diagnose = input.diagnose ?? {}
  const expectedAnimations = input.expectedAnimations ?? null
  const expectedPackageVersion = `${metadata.pkgVersion}-${metadata.pkgRel}`
  const installedPackage = diagnose.installation?.packages?.pacman ?? null
  const binary = diagnose.installation?.binaries?.find(item => item.name === 'lexora-buddy') ?? null
  const socket = diagnose.socket ?? {}
  const sidecarCount = diagnose.runtime?.sidecarCount ?? 0
  const identityErrors = []

  if (!installedPackage) {
    identityErrors.push('installed pacman package lexora-buddy-bin is missing')
  }
  else if (installedPackage.version !== expectedPackageVersion) {
    identityErrors.push(
      `installed pacman package ${installedPackage.name} ${installedPackage.version} does not match expected ${expectedPackageVersion}`,
    )
  }

  if (!binary?.path)
    identityErrors.push('installed lexora-buddy binary is missing from PATH')

  const errors = [...identityErrors]

  if (identityErrors.length > 0) {
    return {
      binary,
      errors,
      expectedPackageVersion,
      installedPackage,
      runtimeAnimationCount: null,
      sidecarCount,
    }
  }

  if (!socket.exists) {
    errors.push('native pet socket does not exist')
  }
  else if (!socket.responsive) {
    errors.push('native pet socket exists but is not responsive')
  }

  if (sidecarCount !== 1)
    errors.push(`expected exactly 1 native pet sidecar, found ${sidecarCount}`)

  const runtimeAnimations = capabilities?.animations
  if (expectedAnimations) {
    if (!Array.isArray(runtimeAnimations)) {
      errors.push('installed runtime capabilities animations are missing')
    }
    else if (!sameValues(runtimeAnimations, expectedAnimations)) {
      const actualAnimationNames = new Set(runtimeAnimations)
      const expectedAnimationNames = new Set(expectedAnimations)
      const missing = expectedAnimations.filter(name => !actualAnimationNames.has(name))
      const unexpected = runtimeAnimations.filter(name => !expectedAnimationNames.has(name))
      const details = []

      if (missing.length > 0)
        details.push(`missing [${missing.join(', ')}]`)
      if (unexpected.length > 0)
        details.push(`unexpected [${unexpected.join(', ')}]`)
      if (details.length === 0)
        details.push('animation order differs')

      errors.push(
        `installed runtime animations do not match current manifest: expected ${expectedAnimations.length}, found ${runtimeAnimations.length}, ${details.join(', ')}`,
      )
    }
  }

  return {
    binary,
    errors,
    expectedPackageVersion,
    installedPackage,
    runtimeAnimationCount: Array.isArray(runtimeAnimations) ? runtimeAnimations.length : null,
    sidecarCount,
  }
}

export function formatBuddyInstalledRuntimeIdentityOutput(result) {
  if (result.errors.length > 0)
    return result.errors.join('\n')

  const packageName = result.installedPackage?.name ?? 'lexora-buddy-bin'
  const binaryPath = result.binary?.path ?? '<missing>'
  const binaryHash = result.binary?.sha256 ?? '<unknown-sha256>'
  const animationSummary = Number.isInteger(result.runtimeAnimationCount)
    ? `, ${result.runtimeAnimationCount} animations`
    : ''

  return `Buddy installed runtime identity check passed: ${packageName} ${result.expectedPackageVersion}, ${binaryPath} ${binaryHash}, ${result.sidecarCount} sidecar${animationSummary}`
}

export function readBuddyReleaseMetadata(cwd = repoRoot) {
  const metadata = createLexoraBuddyReleaseMetadata({
    buddyVersionJson: readFileSync(join(cwd, 'apps/buddy/buddy.version.json'), 'utf8'),
    buddyPackageJson: readFileSync(join(cwd, 'apps/buddy/package.json'), 'utf8'),
    cargoToml: readFileSync(join(cwd, 'apps/buddy/src-tauri/Cargo.toml'), 'utf8'),
    pkgbuild: readFileSync(join(cwd, 'packaging/buddy/aur/lexora-buddy-bin/PKGBUILD'), 'utf8'),
    repoRoot: cwd,
    srcinfo: readFileSync(join(cwd, 'packaging/buddy/aur/lexora-buddy-bin/.SRCINFO'), 'utf8'),
    tauriConfigJson: readFileSync(join(cwd, 'apps/buddy/src-tauri/tauri.conf.json'), 'utf8'),
  })
  const errors = validateLexoraBuddyReleaseMetadata(metadata)

  if (errors.length > 0)
    throw new Error(errors.join('\n'))

  return metadata
}

export function readBuddyInstalledRuntimeDiagnose(cwd = repoRoot, options = {}) {
  return readBuddyInstalledRuntimeCommand('diagnose', cwd, options)
}

export function readBuddyInstalledRuntimeCapabilities(cwd = repoRoot, options = {}) {
  return readBuddyInstalledRuntimeCommand('capabilities', cwd, options)
}

export function readBuddyRuntimeManifestAnimationNames(cwd = repoRoot) {
  const manifest = JSON.parse(readFileSync(
    join(cwd, 'packages/assets/buddy/pets/default/manifest.json'),
    'utf8',
  ))

  return manifest.animations.map(animation => animation.name)
}

function readBuddyInstalledRuntimeCommand(command, cwd, options) {
  const exec = options.execFileSync ?? execFileSync
  const diagnoseScript = join(
    cwd,
    'apps/buddy/src-tauri/skills/lexora-buddy-animation/scripts/lexora-buddy-pet.mjs',
  )
  const output = exec(process.execPath, [diagnoseScript, command], {
    cwd,
    encoding: 'utf8',
  })

  return JSON.parse(output)
}

export function runBuddyInstalledRuntimeIdentityCheck(options = {}) {
  const cwd = options.cwd ?? repoRoot
  const metadata = options.metadata ?? readBuddyReleaseMetadata(cwd)
  const diagnose = options.diagnose ?? readBuddyInstalledRuntimeDiagnose(cwd, options)
  const identityResult = evaluateBuddyInstalledRuntimeIdentity({ diagnose, metadata })

  if (identityResult.errors.length > 0)
    throw new Error(formatBuddyInstalledRuntimeIdentityOutput(identityResult))

  const capabilities = options.capabilities ?? readBuddyInstalledRuntimeCapabilities(cwd, options)
  const expectedAnimations = options.expectedAnimations ?? readBuddyRuntimeManifestAnimationNames(cwd)
  const result = evaluateBuddyInstalledRuntimeIdentity({
    capabilities,
    diagnose,
    expectedAnimations,
    metadata,
  })

  if (result.errors.length > 0)
    throw new Error(formatBuddyInstalledRuntimeIdentityOutput(result))

  writeOutput(formatBuddyInstalledRuntimeIdentityOutput(result))
  return result
}

function sameValues(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index])
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    runBuddyInstalledRuntimeIdentityCheck()
  }
  catch (error) {
    writeError(error.message)
    process.exit(1)
  }
}
