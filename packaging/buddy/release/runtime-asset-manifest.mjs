import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')

export function evaluateBuddyRuntimeAssetManifest(manifest) {
  const errors = []
  const animations = Array.isArray(manifest.animations) ? manifest.animations : []
  const columns = manifest.sheet?.columns

  if (!Number.isInteger(columns) || columns <= 0)
    errors.push('manifest.sheet.columns must be a positive integer')
  if (!Array.isArray(manifest.animations))
    errors.push('manifest.animations must be an array')

  const animationsByName = new Map(animations.map(animation => [animation.name, animation]))

  if (errors.length === 0)
    validateSleepWakeRuntimeManifest({ animationsByName, columns, errors })

  return {
    animationCount: animations.length,
    columns,
    errors,
  }
}

export function formatBuddyRuntimeAssetManifestOutput(result) {
  if (result.errors.length > 0)
    return result.errors.join('\n')

  return `Buddy runtime asset manifest check passed: ${result.animationCount} animations, ${result.columns} columns`
}

export function runBuddyRuntimeAssetManifestCheck(options = {}) {
  const cwd = options.cwd ?? repoRoot
  const manifest = options.manifest ?? JSON.parse(readFileSync(
    join(cwd, 'packages/assets/buddy/pets/default/manifest.json'),
    'utf8',
  ))
  const result = evaluateBuddyRuntimeAssetManifest(manifest)

  if (result.errors.length > 0)
    throw new Error(formatBuddyRuntimeAssetManifestOutput(result))

  return result
}

function validateSleepWakeRuntimeManifest({ animationsByName, columns, errors }) {
  const sleepEnter = animationsByName.get('sleep_enter')
  const sleep = animationsByName.get('sleep')
  const wake = animationsByName.get('wake')
  const lifecycleAnimations = [sleepEnter, sleep, wake]

  for (const [name, animation] of [
    ['sleep_enter', sleepEnter],
    ['sleep', sleep],
    ['wake', wake],
  ]) {
    if (!animation)
      errors.push(`runtime animation "${name}" is missing`)
    else if (!Number.isInteger(animation.row))
      errors.push(`runtime animation "${name}" must declare row`)
  }

  if (lifecycleAnimations.some(animation => !animation || !Number.isInteger(animation.row)))
    return

  const row = sleepEnter.row
  for (const animation of [sleep, wake]) {
    if (animation.row !== row)
      errors.push(`runtime animation "${animation.name}" must share row ${row} with sleep_enter`)
  }

  assertAnimationOffsets({ animation: sleepEnter, columns, errors, expectedOffsets: [0, 1, 2, 3] })
  assertAnimationOffsets({ animation: sleep, columns, errors, expectedOffsets: [3] })
  assertAnimationOffsets({ animation: wake, columns, errors, expectedOffsets: [4, 5, 6, 7] })
  assertLoop({ animation: sleepEnter, errors, expectedLoop: false })
  assertLoop({ animation: sleep, errors, expectedLoop: true })
  assertLoop({ animation: wake, errors, expectedLoop: false })
}

function assertAnimationOffsets({ animation, columns, errors, expectedOffsets }) {
  const offsets = animationOffsets(animation, columns)

  if (!sameValues(offsets, expectedOffsets))
    errors.push(`runtime animation "${animation.name}" must use source offsets [${expectedOffsets.join(', ')}]`)
}

function assertLoop({ animation, errors, expectedLoop }) {
  if (animation.loop !== expectedLoop)
    errors.push(`runtime animation "${animation.name}" must set loop=${expectedLoop}`)
}

function animationOffsets(animation, columns) {
  if (!Array.isArray(animation.frames))
    return []

  return animation.frames.map(frame => frameIndex(frame) - animation.row * columns)
}

function frameIndex(frame) {
  return typeof frame === 'number' ? frame : frame?.index
}

function sameValues(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index])
}
