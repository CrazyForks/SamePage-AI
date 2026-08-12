import { execFileSync } from 'node:child_process'
import { resolve } from 'node:path'
import process from 'node:process'

import { writeOutput } from '../../shared/cli-output.mjs'

const repoRoot = resolve(import.meta.dirname, '../../..')

export function createBuddyReleasePreflightSteps() {
  return [
    ['Version consistency', 'node', ['packaging/buddy/release/buddy-version.mjs', '--check']],
    ['Delivery scope', 'node', ['packaging/buddy/release/verify-delivery-scope.mjs']],
    ['Linux release workflow', 'node', ['packaging/buddy/release/verify-linux-release-workflow.mjs']],
    ['Desktop source lint', 'pnpm', ['exec', 'eslint', 'apps/buddy', 'packaging/buddy']],
    ['Desktop type-check', 'pnpm', ['--filter', '@lexora/buddy', 'type-check']],
    ['Desktop tests', 'pnpm', ['--filter', '@lexora/buddy', 'test']],
    ['Packaging contract tests', 'pnpm', ['exec', 'vitest', 'run', 'packaging/buddy/__tests__/preflight.spec.mjs', 'packaging/buddy/__tests__/deliveryScope.spec.mjs', '--passWithNoTests']],
    ['AUR package contract', 'node', ['packaging/buddy/aur/verify-bin-package.mjs']],
    ['Runtime format', 'cargo', ['fmt', '--manifest-path', 'apps/buddy/runtime/Cargo.toml', '--', '--check']],
    ['Runtime check', 'cargo', ['check', '--manifest-path', 'apps/buddy/runtime/Cargo.toml']],
    ['Pet-only check', 'cargo', ['check', '--manifest-path', 'apps/buddy/runtime/Cargo.toml', '--no-default-features', '--features', 'pet', '--all-targets']],
    ['Runtime clippy', 'cargo', ['clippy', '--manifest-path', 'apps/buddy/runtime/Cargo.toml', '--all-targets', '--all-features', '--', '-D', 'warnings']],
    ['Pet-only clippy', 'cargo', ['clippy', '--manifest-path', 'apps/buddy/runtime/Cargo.toml', '--no-default-features', '--features', 'pet', '--all-targets', '--', '-D', 'warnings']],
    ['Runtime tests', 'cargo', ['test', '--manifest-path', 'apps/buddy/runtime/Cargo.toml', '--lib']],
    ['Pet-only tests', 'cargo', ['test', '--manifest-path', 'apps/buddy/runtime/Cargo.toml', '--no-default-features', '--features', 'pet', '--lib']],
    ['Staged whitespace', 'git', ['diff', '--cached', '--check']],
    ['Workspace whitespace', 'git', ['diff', '--check']],
    ['Electron build', 'pnpm', ['--filter', '@lexora/buddy', 'exec', 'electron-vite', 'build']],
    ['Electron bundle boundary', 'node', ['packaging/buddy/release/verify-electron-bundle.mjs']],
    ['Standalone pet package', 'pnpm', ['--filter', '@lexora/buddy', 'package:pet']],
    ['Standalone pet contract test', 'pnpm', ['exec', 'vitest', 'run', 'packaging/buddy/__tests__/petBinaryBoundary.spec.mjs', '--passWithNoTests']],
    ['Full Desktop deb package', 'pnpm', ['--filter', '@lexora/buddy', 'exec', 'electron-builder', '--config', 'electron-builder.config.cjs', '--linux', 'deb']],
    ['Deb Runtime dependencies', 'node', ['packaging/buddy/release/verify-deb-dependencies.mjs']],
    ['AUR release artifact', 'node', ['packaging/buddy/ci/verify-linux-deb-artifact.mjs']],
  ].map(([label, command, args]) => ({ label, command, args }))
}

export function runBuddyReleasePreflight(options = {}) {
  const cwd = options.cwd ?? repoRoot
  const env = options.env ?? process.env

  for (const step of createBuddyReleasePreflightSteps()) {
    writeOutput(`\n[Buddy] ${step.label}`)
    execFileSync(step.command, step.args, {
      cwd,
      env: {
        ...env,
        RUSTFLAGS: env.RUSTFLAGS ?? '-D warnings',
        RUST_MIN_STACK: env.RUST_MIN_STACK ?? '16777216',
      },
      stdio: 'inherit',
    })
  }

  writeOutput('\nLexora Desktop preflight passed')
}

if (process.argv[1] && resolve(process.argv[1]) === new URL(import.meta.url).pathname)
  runBuddyReleasePreflight()
