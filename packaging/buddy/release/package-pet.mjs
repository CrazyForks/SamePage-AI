import { spawnSync } from 'node:child_process'
import { chmodSync, cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'
import process from 'node:process'

import { writeOutput } from '../../shared/cli-output.mjs'
import { assertPetBinaryBoundary } from './verify-pet-binary-boundary.mjs'

const repoRoot = resolve(import.meta.dirname, '../../..')
const buddyRoot = join(repoRoot, 'apps/buddy')
const version = JSON.parse(
  readFileSync(join(buddyRoot, 'buddy.version.json'), 'utf8'),
).version
const architecture = normalizeArchitecture(process.arch)
const packageName = `Lexora-Pet-${version}-linux-${architecture}`
const outputRoot = join(buddyRoot, 'dist-packages')
const stagingRoot = join(outputRoot, 'pet-only')
const packageRoot = join(stagingRoot, packageName)
const runtimeSource = join(buddyRoot, 'runtime/target/release/lexora-buddy-runtime')
const petSource = join(buddyRoot, 'runtime/target/release/lexora-buddy-pet')
const petTarget = join(packageRoot, 'bin/lexora-buddy-pet')

if (process.platform !== 'linux')
  throw new Error('Lexora Pet standalone packaging currently supports Linux only')

rmSync(stagingRoot, { force: true, recursive: true })
assertPetBinaryBoundary({ petPath: petSource, runtimePath: runtimeSource })
mkdirSync(join(packageRoot, 'bin'), { recursive: true })
mkdirSync(join(packageRoot, 'share/applications'), { recursive: true })
mkdirSync(join(packageRoot, 'share/icons/hicolor/256x256/apps'), { recursive: true })

cpSync(petSource, petTarget)
chmodSync(petTarget, 0o755)
cpSync(
  join(buddyRoot, 'runtime/icons/icon.png'),
  join(packageRoot, 'share/icons/hicolor/256x256/apps/lexora-buddy.png'),
)
writeFileSync(
  join(packageRoot, 'share/applications/lexora-buddy.desktop'),
  [
    '[Desktop Entry]',
    'Type=Application',
    'Name=Lexora Buddy',
    'Comment=Lexora desktop pet',
    'Exec=lexora-buddy-pet --native-pet',
    'Icon=lexora-buddy',
    'Terminal=false',
    'Categories=Utility;',
    '',
  ].join('\n'),
)
writeFileSync(
  join(packageRoot, 'README.txt'),
  [
    'Lexora Buddy standalone pet',
    '',
    'Copy bin/lexora-buddy-pet into PATH, then run:',
    '  lexora-buddy-pet --native-pet',
    '',
    'This package contains only the native pet. It does not include Lexora Desktop or Codex chat.',
    '',
  ].join('\n'),
)

mkdirSync(outputRoot, { recursive: true })
const archivePath = join(outputRoot, `${packageName}.tar.gz`)
const result = spawnSync('tar', [
  '-czf',
  archivePath,
  '-C',
  stagingRoot,
  basename(packageRoot),
], { stdio: 'inherit' })
if (result.status !== 0)
  throw new Error(`tar failed with exit code ${result.status ?? 'unknown'}`)

writeOutput(archivePath)

function normalizeArchitecture(architecture) {
  if (architecture === 'x64')
    return 'x86_64'
  if (architecture === 'arm64')
    return 'aarch64'
  return architecture
}
