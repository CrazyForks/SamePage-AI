import { spawnSync } from 'node:child_process'
import { readdir, readFile, stat } from 'node:fs/promises'
import { resolve } from 'node:path'
import process from 'node:process'

import { writeError, writeOutput } from '../../shared/cli-output.mjs'

const repoRoot = resolve(import.meta.dirname, '../../..')
const buddyRoot = resolve(repoRoot, 'apps/buddy')
const requiredDependencies = ['libgtk-3-0', 'libgtk-layer-shell0']

async function main() {
  const packageJson = JSON.parse(await readFile(resolve(buddyRoot, 'package.json'), 'utf8'))
  const artifact = await findLatestArtifact(packageJson.version)
  const dependencies = parseDependencies(extractControl(artifact))
  const missingDependencies = requiredDependencies.filter(dependency => !dependencies.has(dependency))

  if (missingDependencies.length)
    throw new Error(`Deb package is missing Runtime dependencies: ${missingDependencies.join(', ')}`)

  writeOutput(`Verified Deb Runtime dependencies: ${requiredDependencies.join(', ')}`)
}

async function findLatestArtifact(version) {
  const outputDirectory = resolve(buddyRoot, 'dist-packages')
  const candidates = (await readdir(outputDirectory))
    .filter(name => name.startsWith(`Lexora-${version}-linux-`) && name.endsWith('.deb'))
    .map(name => resolve(outputDirectory, name))

  if (candidates.length === 0)
    throw new Error(`No Lexora ${version} Deb artifact found in ${outputDirectory}`)

  const withStats = await Promise.all(candidates.map(async path => ({
    path,
    modifiedAt: (await stat(path)).mtimeMs,
  })))
  withStats.sort((left, right) => right.modifiedAt - left.modifiedAt)
  return withStats[0].path
}

function extractControl(artifact) {
  const members = run('ar', ['t', artifact]).trim().split(/\r?\n/)
  const controlArchive = members.find(member => /^control\.tar\.(?:xz|gz|zst)$/.test(member))
  if (!controlArchive)
    throw new Error(`Deb control archive is missing: ${artifact}`)

  const archive = runBuffer('ar', ['p', artifact, controlArchive])
  const compressionArgs = controlArchive.endsWith('.xz')
    ? ['-xJOf']
    : controlArchive.endsWith('.gz') ? ['-xzOf'] : ['--zstd', '-xOf']
  return run('tar', [...compressionArgs, '-', './control'], archive)
}

function parseDependencies(control) {
  const lines = control.split(/\r?\n/)
  const fieldIndex = lines.findIndex(line => line.startsWith('Depends:'))
  if (fieldIndex < 0)
    return new Set()

  const fieldLines = [lines[fieldIndex].slice('Depends:'.length)]
  for (let index = fieldIndex + 1; index < lines.length && /^[ \t]/.test(lines[index]); index++)
    fieldLines.push(lines[index].trim())

  return new Set(fieldLines.join(' ')
    .split(',')
    .map(value => value.trim().split(/[ (|]/, 1)[0])
    .filter(Boolean))
}

function run(command, args, input) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    input,
    maxBuffer: 8 * 1024 * 1024,
  })
  if (result.status !== 0)
    throw new Error(result.stderr || `${command} exited with ${result.status ?? 'unknown status'}`)

  return result.stdout
}

function runBuffer(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    maxBuffer: 8 * 1024 * 1024,
  })
  if (result.status !== 0)
    throw new Error(result.stderr.toString() || `${command} exited with ${result.status ?? 'unknown status'}`)

  return result.stdout
}

if (process.argv[1] && resolve(process.argv[1]) === new URL(import.meta.url).pathname) {
  void main().catch((error) => {
    writeError(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  })
}
