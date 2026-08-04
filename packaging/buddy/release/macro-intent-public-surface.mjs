import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')

const boundSpecs = [
  {
    key: 'danceDurationMs',
    label: 'dance.durationMs',
    rustMax: 'PUBLIC_DANCE_DURATION_MS_MAX',
    rustMin: 'PUBLIC_DANCE_DURATION_MS_MIN',
    tsPath: ['dance', 'durationMs'],
  },
  {
    key: 'patrolAroundScreenLoops',
    label: 'patrolAroundScreen.loops',
    rustMax: 'PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MAX',
    rustMin: 'PUBLIC_PATROL_AROUND_SCREEN_LOOPS_MIN',
    tsPath: ['patrolAroundScreen', 'loops'],
  },
  {
    key: 'peekBehindWindowDurationMs',
    label: 'peekBehindWindow.durationMs',
    rustMax: 'PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MAX',
    rustMin: 'PUBLIC_PEEK_BEHIND_WINDOW_DURATION_MS_MIN',
    tsPath: ['peekBehindWindow', 'durationMs'],
  },
]

export function evaluateBuddyMacroIntentPublicSurface(input) {
  const rustMacroIds = parseRustStringArrayConst(input.rustMacroPlan, 'PUBLIC_MACRO_INTENT_IDS')
  const tsMacroIds = parseTsStringArrayConst(input.tsHostAction, 'BUDDY_HOST_ACTION_PUBLIC_MACRO_IDS')
  const errors = []

  if (!arraysEqual(tsMacroIds, rustMacroIds)) {
    errors.push('frontend BUDDY_HOST_ACTION_PUBLIC_MACRO_IDS does not match Rust PUBLIC_MACRO_INTENT_IDS')
    errors.push(`frontend macro ids: ${tsMacroIds.join(', ')}`)
    errors.push(`rust macro ids: ${rustMacroIds.join(', ')}`)
  }

  const paramBounds = {}
  for (const spec of boundSpecs) {
    const rustMin = parseRustNumberConst(input.rustMacroPlan, spec.rustMin)
    const rustMax = parseRustNumberConst(input.rustMacroPlan, spec.rustMax)
    const tsBounds = parseTsNestedNumberBounds(input.tsHostAction, spec.tsPath)

    paramBounds[spec.key] = { max: rustMax, min: rustMin }

    if (tsBounds.min !== rustMin) {
      errors.push(
        `frontend ${spec.label} min ${tsBounds.min} does not match Rust ${spec.rustMin} ${rustMin}`,
      )
    }
    if (tsBounds.max !== rustMax) {
      errors.push(
        `frontend ${spec.label} max ${tsBounds.max} does not match Rust ${spec.rustMax} ${rustMax}`,
      )
    }
  }

  return {
    errors,
    macroIds: rustMacroIds,
    paramBounds,
  }
}

export function formatBuddyMacroIntentPublicSurfaceOutput(result) {
  if (result.errors.length > 0)
    return result.errors.join('\n')

  return `Buddy macroIntent public surface check passed: ${result.macroIds.length} macro ids, ${Object.keys(result.paramBounds).length} parameter bounds`
}

export function runBuddyMacroIntentPublicSurfaceCheck(options = {}) {
  const cwd = options.cwd ?? repoRoot
  const result = evaluateBuddyMacroIntentPublicSurface({
    rustMacroPlan: options.rustMacroPlan ?? readFileSync(
      join(cwd, 'apps/buddy/src-tauri/src/choreography/macro_plan.rs'),
      'utf8',
    ),
    tsHostAction: options.tsHostAction ?? readFileSync(
      join(cwd, 'apps/buddy/src/pet/buddyHostAction.ts'),
      'utf8',
    ),
  })

  if (result.errors.length > 0)
    throw new Error(formatBuddyMacroIntentPublicSurfaceOutput(result))

  return result
}

function parseRustStringArrayConst(source, name) {
  const match = source.match(new RegExp(`const\\s+${escapeRegExp(name)}\\s*:[^=]+?=\\s*&\\[([\\s\\S]*?)\\];`))
  if (!match)
    throw new Error(`Rust const ${name} is missing`)

  return [...match[1].matchAll(/"([^"]+)"/g)].map(item => item[1])
}

function parseTsStringArrayConst(source, name) {
  const match = source.match(new RegExp(`const\\s+${escapeRegExp(name)}\\s*=\\s*\\[([\\s\\S]*?)\\]\\s*as\\s+const`))
  if (!match)
    throw new Error(`TypeScript const ${name} is missing`)

  return [...match[1].matchAll(/['"]([^'"]+)['"]/g)].map(item => item[1])
}

function parseRustNumberConst(source, name) {
  const match = source.match(new RegExp(`const\\s+${escapeRegExp(name)}\\s*:[^=]+?=\\s*([\\d_]+)\\s*;`))
  if (!match)
    throw new Error(`Rust numeric const ${name} is missing`)

  return parseUnderscoreInt(match[1])
}

function parseTsNestedNumberBounds(source, path) {
  let block = source
  for (const property of path)
    block = extractTsObjectPropertyBlock(block, property)

  return {
    max: parseTsNumberProperty(block, 'max'),
    min: parseTsNumberProperty(block, 'min'),
  }
}

function extractTsObjectPropertyBlock(source, propertyName) {
  const propertyMatch = new RegExp(`${escapeRegExp(propertyName)}\\s*:`).exec(source)
  if (!propertyMatch)
    throw new Error(`TypeScript object property ${propertyName} is missing`)

  const openBraceIndex = source.indexOf('{', propertyMatch.index + propertyMatch[0].length)
  if (openBraceIndex === -1)
    throw new Error(`TypeScript object property ${propertyName} has no object value`)

  let depth = 0
  for (let index = openBraceIndex; index < source.length; index += 1) {
    if (source[index] === '{')
      depth += 1
    if (source[index] === '}')
      depth -= 1
    if (depth === 0)
      return source.slice(openBraceIndex + 1, index)
  }

  throw new Error(`TypeScript object property ${propertyName} object is not closed`)
}

function parseTsNumberProperty(source, name) {
  const match = source.match(new RegExp(`${escapeRegExp(name)}\\s*:\\s*([\\d_]+)`))
  if (!match)
    throw new Error(`TypeScript numeric property ${name} is missing`)

  return parseUnderscoreInt(match[1])
}

function parseUnderscoreInt(value) {
  return Number.parseInt(value.replaceAll('_', ''), 10)
}

function arraysEqual(left, right) {
  return left.length === right.length && left.every((item, index) => item === right[index])
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
