import { spawn } from 'node:child_process'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import process from 'node:process'

import { writeError, writeOutput } from '../../shared/cli-output.mjs'

const repoRoot = resolve(import.meta.dirname, '../../..')
const guiRunner = process.env.LEXORA_GUI_SMOKE_RUNNER ?? 'xvfb-run'
async function main() {
  const desktopPath = resolve(
    process.env.LEXORA_DESKTOP_EXECUTABLE_PATH
    ?? resolve(repoRoot, 'apps/buddy/dist-packages/linux-unpacked/lexora'),
  )
  const binaryPath = resolve(
    process.env.LEXORA_BUDDY_PET_PATH
    ?? resolve(repoRoot, 'apps/buddy/runtime/target/release/lexora-buddy-pet'),
  )
  const smokeRoot = mkdtempSync(join(tmpdir(), 'lexora-desktop-smoke-'))
  const smokeEnv = {
    ...process.env,
    LEXORA_BUDDY_PET_SOCKET: join(smokeRoot, 'native-pet.sock'),
    LEXORA_HOME: join(smokeRoot, 'home'),
  }

  await runDesktopSmoke(desktopPath, smokeEnv)
  await runNativePetSmoke(binaryPath, 12_000, smokeEnv)
  writeOutput('Lexora Desktop and standalone pet GUI smoke passed')
}

export function runDesktopSmoke(executablePath, env, timeoutMs = 30_000) {
  return new Promise((resolveSmoke, rejectSmoke) => {
    const child = spawnGui(executablePath, {
      ...env,
      LEXORA_DESKTOP_SMOKE_TEST: '1',
    })
    let stderr = ''
    const timeout = setTimeout(() => {
      child.kill('SIGKILL')
      rejectSmoke(new Error(`Desktop smoke did not exit within ${timeoutMs}ms`))
    }, timeoutMs)

    child.stderr.setEncoding('utf8')
    child.stderr.on('data', (chunk) => {
      stderr = `${stderr}${chunk}`.slice(-8_192)
    })
    child.on('error', (error) => {
      clearTimeout(timeout)
      rejectSmoke(error)
    })
    child.on('exit', (code, signal) => {
      clearTimeout(timeout)
      if (code === 0) {
        resolveSmoke()
        return
      }

      rejectSmoke(new Error(`Desktop smoke failed: ${signal ?? code}; ${stderr.trim()}`))
    })
  })
}

export function runNativePetSmoke(runtimePath, timeoutMs = 12_000, env = process.env) {
  return new Promise((resolveSmoke, rejectSmoke) => {
    const child = spawnGui(runtimePath, env, ['--native-pet'])
    let settled = false
    let ready = false
    let stdout = ''
    let stderr = ''
    const timeout = setTimeout(() => {
      finish(new Error(`native pet did not become ready within ${timeoutMs}ms`))
    }, timeoutMs)

    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', (chunk) => {
      stdout = `${stdout}${chunk}`.slice(-8_192)
      if (stdout.includes('event:ready') && !ready) {
        ready = true
        child.kill('SIGTERM')
      }
    })
    child.stderr.on('data', (chunk) => {
      stderr = `${stderr}${chunk}`.slice(-2_048)
    })
    child.on('error', finish)
    child.on('exit', (code, signal) => {
      if (!settled) {
        finish(ready
          ? undefined
          : new Error(`native pet exited before ready: ${signal ?? code}; ${stderr.trim()}`))
      }
    })

    function finish(error) {
      if (settled)
        return

      settled = true
      clearTimeout(timeout)
      if (!ready)
        child.kill('SIGTERM')
      if (error)
        rejectSmoke(error)
      else
        resolveSmoke()
    }
  })
}

function spawnGui(executablePath, env, args = []) {
  const command = guiRunner === 'direct' ? executablePath : guiRunner
  const commandArgs = guiRunner === 'direct'
    ? args
    : ['-a', executablePath, ...args]

  return spawn(command, commandArgs, {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
}

if (process.argv[1] && resolve(process.argv[1]) === new URL(import.meta.url).pathname) {
  void main().catch((error) => {
    writeError(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  })
}
