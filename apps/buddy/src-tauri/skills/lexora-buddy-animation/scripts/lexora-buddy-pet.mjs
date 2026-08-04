#!/usr/bin/env node

import { execFileSync, spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import process from 'node:process'

const socketEnvName = 'LEXORA_BUDDY_PET_SOCKET'
const defaultSocketPath = createDefaultSocketPath()
const socketPath = process.env[socketEnvName] || defaultSocketPath
const usesCustomSocketPath = Boolean(process.env[socketEnvName])
const connectTimeoutMs = 1_500
const launchWaitMs = 5_000
const buddyBinaries = ['lexora-buddy-pet', 'lexora-buddy']
const kwinActiveWindowOutputPrefix = 'lexora-buddy-active-window:'
const kwinActiveWindowQueryTimeoutMs = 650
const kwinActiveWindowPollIntervalMs = 50

function createDefaultSocketPath() {
  const runtimeDir = process.env.XDG_RUNTIME_DIR
  if (runtimeDir)
    return path.join(runtimeDir, 'lexora-buddy', 'native-pet.sock')

  return path.join(os.tmpdir(), `lexora-buddy-uid-${resolveProcessUidSegment()}`, 'native-pet.sock')
}

function resolveProcessUidSegment() {
  return typeof process.getuid === 'function' ? process.getuid() : 'user'
}

const animationAliases = new Map([
  ['focus', 'working'],
  ['left', 'run_left'],
  ['right', 'run_right'],
])
let runtimeAnimationsPromise

async function main() {
  const [command, ...rest] = process.argv.slice(2)
  try {
    switch (command) {
      case 'diagnose':
        printJson(await diagnose())
        return
      case 'socket-path':
        process.stdout.write(`${socketPath}\n`)
        return
      case 'active-window':
        printJson(detectActiveWindow())
        return
      case 'sidecars':
        printJson({ ok: true, sidecars: detectNativePetSidecars() })
        return
      case 'launch':
        printJson(await launchBuddyPet())
        return
      case 'state':
        printJson(await readPetState())
        return
      case 'capabilities':
        printJson(await sendControlRequest({ type: 'capabilities' }))
        return
      case 'animation':
        await sendAnimation(requiredArg(rest, 'animation name'))
        return
      case 'move':
        await sendMoveCommand(rest)
        return
      case 'perform':
        await performPreset(requiredArg(rest, 'preset name'), parseOptions(rest.slice(1)))
        return
      case 'sequence':
        await runSequenceCommand(parseOptions(rest))
        return
      case 'walk-left':
      case 'walk-window-left':
        await sendWindowWalk('left', parseOptions(rest))
        return
      case 'walk-right':
      case 'walk-window-right':
        await sendWindowWalk('right', parseOptions(rest))
        return
      case 'walk-up':
      case 'walk-window-up':
        await sendWindowWalk('top', parseOptions(rest))
        return
      case 'walk-down':
      case 'walk-window-down':
        await sendWindowWalk('bottom', parseOptions(rest))
        return
      case 'walk-to-edge':
        await sendWalkToEdge(requiredArg(rest, 'edge'), parseOptions(rest.slice(1)))
        return
      case 'walk-to-x':
        await sendWalkToX(readRequiredNumberOption(parseOptions(rest), 'x'), parseOptions(rest))
        return
      case 'walk-to':
        await sendWalkToPosition(parseOptions(rest))
        return
      case 'help':
      case undefined:
        printHelp()
        return
      default:
        throw new Error(`unknown command: ${command}`)
    }
  }
  catch (error) {
    printJson({
      ok: false,
      error: error instanceof Error ? error.message : String(error),
      socketPath,
    })
    process.exitCode = 1
  }
}

async function sendAnimation(name) {
  const { animation } = await sendAnimationRequest(name)
  printJson({ ok: true, command: 'animation', animation, socketPath })
}

async function sendAnimationRequest(name) {
  const animation = await normalizeAnimation(name)
  await sendControlRequest({ type: 'animation', animation })
  return { animation }
}

async function sendWindowWalk(edge, options) {
  const after = await readOptionalAnimation(options.after)
  const activeWindow = detectActiveWindow()
  if ((edge === 'left' || edge === 'right') && activeWindow.ok && Number.isFinite(activeWindow.x)) {
    const x = edge === 'left' ? activeWindow.x : activeWindow.x + activeWindow.width
    await sendMoveTarget({ kind: 'x', x: Math.round(x) }, after)
    printJson({ ok: true, command: `walk-window-${edge}`, target: 'active-window', activeWindow, after, socketPath })
    return
  }

  await sendWalkToEdge(edge, options, activeWindow)
}

async function sendWalkToEdge(edge, options, activeWindow = detectActiveWindow()) {
  edge = normalizeEdge(edge)

  const after = await readOptionalAnimation(options.after)
  await sendMoveTarget({ kind: 'edge', edge }, after)
  printJson({ ok: true, command: `walk-${edge}`, target: 'screen-edge', activeWindow, after, socketPath })
}

async function sendWalkToX(x, options) {
  const after = await readOptionalAnimation(options.after)
  await sendMoveTarget({ kind: 'x', x: Math.round(x) }, after)
  printJson({ ok: true, command: 'walk-to-x', x: Math.round(x), after, socketPath })
}

async function sendWalkToPosition(options) {
  const x = readRequiredNumberOption(options, 'x')
  const y = readRequiredNumberOption(options, 'y')
  const after = await readOptionalAnimation(options.after)
  await sendMoveTarget({ kind: 'position', x: Math.round(x), y: Math.round(y) }, after)
  printJson({ ok: true, command: 'walk-to', x: Math.round(x), y: Math.round(y), after, socketPath })
}

async function sendMoveCommand(args) {
  const target = requiredArg(args, 'move target')
  const options = parseOptions(args.slice(1))
  const after = await readOptionalAnimation(options.after)
  switch (target) {
    case 'center':
      await sendMoveTarget({ kind: 'center' }, after)
      printJson({ ok: true, command: 'move', target: 'center', after, socketPath })
      return
    case 'home':
      await sendMoveTarget({ kind: 'home' }, after)
      printJson({ ok: true, command: 'move', target: 'home', after, socketPath })
      return
    case 'edge': {
      const edge = normalizeEdge(requiredArg(args.slice(1), 'edge'))
      await sendMoveTarget({ kind: 'edge', edge }, after)
      printJson({ ok: true, command: 'move', target: 'edge', edge, after, socketPath })
      return
    }
    case 'position': {
      const x = readRequiredNumberOption(options, 'x')
      const y = readRequiredNumberOption(options, 'y')
      await sendMoveTarget({ kind: 'position', x: Math.round(x), y: Math.round(y) }, after)
      printJson({ ok: true, command: 'move', target: 'position', x: Math.round(x), y: Math.round(y), after, socketPath })
      return
    }
    default:
      throw new Error(`unknown move target: ${target}`)
  }
}

async function sendMoveTarget(target, after) {
  const request = { type: 'move', target }
  if (after)
    request.after = after
  return sendControlRequest(request)
}

async function performPreset(preset, options) {
  switch (preset) {
    case 'center-cast-return-sleep':
      await performCenterCastReturnSleep(options)
      return
    default:
      throw new Error(`unknown preset: ${preset}`)
  }
}

async function runSequenceCommand(options) {
  const sequence = readSequenceSpec(options)
  const result = await executeSequence(sequence)
  printJson({
    ok: true,
    command: 'sequence',
    steps: result.steps,
    snapshots: Object.fromEntries(result.snapshots),
    socketPath,
  })
}

function readSequenceSpec(options) {
  if (options.json)
    return JSON.parse(options.json)
  if (options.file)
    return JSON.parse(fs.readFileSync(options.file, 'utf8'))
  throw new Error('missing --json or --file for sequence')
}

async function executeSequence(sequence) {
  const steps = Array.isArray(sequence?.steps) ? sequence.steps : sequence
  if (!Array.isArray(steps))
    throw new Error('sequence must be an array or an object with steps')

  const snapshots = new Map()
  for (const step of steps) {
    await executeSequenceStep(step, snapshots)
  }

  return { steps: steps.length, snapshots }
}

async function executeSequenceStep(step, snapshots) {
  switch (step?.type) {
    case 'snapshot': {
      const name = step.name || 'original'
      const state = await readPetState()
      assertPetState(state)
      snapshots.set(name, state.position)
      return
    }
    case 'move': {
      const target = resolveSequenceMoveTarget(step.target, snapshots)
      const after = await readOptionalAnimation(step.after)
      await sendMoveTarget(target, after)
      if (step.wait !== false)
        await waitForMotionIdle(`move:${target.kind}`, readDurationMs(step.timeoutMs, 10_000))
      return
    }
    case 'animation': {
      await sendAnimationRequest(step.animation)
      if (step.durationMs !== undefined)
        await delay(readDurationMs(step.durationMs, 0))
      return
    }
    case 'wait': {
      if (step.motionIdle)
        await waitForMotionIdle('wait', readDurationMs(step.timeoutMs, 10_000))
      else
        await delay(readDurationMs(step.durationMs, 0))
      return
    }
    default:
      throw new Error(`unknown sequence step type: ${step?.type}`)
  }
}

function resolveSequenceMoveTarget(target, snapshots) {
  if (typeof target === 'string') {
    if (target === 'center' || target === 'home')
      return { kind: target }
    throw new Error(`unsupported sequence move target: ${target}`)
  }

  switch (target?.kind) {
    case 'center':
    case 'home':
      return { kind: target.kind }
    case 'edge':
      return { kind: 'edge', edge: normalizeEdge(target.edge) }
    case 'windowAnchor':
      return {
        kind: 'windowAnchor',
        selector: normalizeWindowAnchorSelector(target.selector),
        edge: normalizeWindowAnchorEdge(target.edge),
        reveal: normalizeWindowAnchorReveal(target.reveal),
        durationMs: readWindowAnchorDurationMs(target.durationMs),
      }
    case 'position': {
      const x = Number(target.x)
      const y = Number(target.y)
      if (!Number.isFinite(x) || !Number.isFinite(y))
        throw new Error('sequence position target requires numeric x/y')
      return { kind: 'position', x: Math.round(x), y: Math.round(y) }
    }
    case 'snapshot': {
      const name = target.name || 'original'
      const position = snapshots.get(name)
      if (!position)
        throw new Error(`unknown sequence snapshot: ${name}`)
      return { kind: 'position', x: position.x, y: position.y }
    }
    default:
      throw new Error(`unsupported sequence move target: ${JSON.stringify(target)}`)
  }
}

function normalizeWindowAnchorSelector(selector) {
  if (selector?.kind !== 'activeWindow')
    throw new Error('windowAnchor target selector.kind must be activeWindow')
  return { kind: 'activeWindow' }
}

function normalizeWindowAnchorEdge(value) {
  const edge = String(value || 'auto').trim().toLowerCase()
  if (edge === 'auto')
    return 'auto'
  return normalizeEdge(edge)
}

function normalizeWindowAnchorReveal(value) {
  const reveal = String(value || 'head').trim().toLowerCase()
  if (reveal !== 'head')
    throw new Error(`unsupported windowAnchor reveal: ${value}`)
  return reveal
}

function readWindowAnchorDurationMs(value) {
  const durationMs = readDurationMs(value, 1_500)
  if (durationMs < 500 || durationMs > 15_000)
    throw new Error('windowAnchor durationMs must be between 500 and 15000')
  return durationMs
}

async function performCenterCastReturnSleep(options) {
  const animation = await normalizeAnimation(options.animation || options.cast || 'celebrate')
  const durationMs = readDurationMs(options['duration-ms'], 2_000)
  const waitTimeoutMs = readDurationMs(options['wait-timeout-ms'], 10_000)
  const original = await readPetState()
  assertPetState(original)

  await sendMoveTarget({ kind: 'center' })
  await waitForMotionIdle('center', waitTimeoutMs)
  await sendAnimationRequest(animation)
  await delay(durationMs)
  await sendMoveTarget({
    kind: 'position',
    x: original.position.x,
    y: original.position.y,
  }, 'sleep')
  await waitForMotionIdle('original', waitTimeoutMs)

  printJson({
    ok: true,
    command: 'perform',
    preset: 'center-cast-return-sleep',
    original: original.position,
    animation,
    durationMs,
    socketPath,
  })
}

async function readPetState(candidatePath = socketPath) {
  return sendControlRequest({ type: 'state' }, candidatePath)
}

function assertPetState(state) {
  if (!state?.ok)
    throw new Error(state?.error || 'Lexora Buddy pet did not return state')
  if (!Number.isFinite(state.position?.x) || !Number.isFinite(state.position?.y))
    throw new Error('Lexora Buddy pet state is missing position')
}

async function waitForMotionIdle(label, timeoutMs) {
  const startedAt = Date.now()
  while (Date.now() - startedAt < timeoutMs) {
    const state = await readPetState()
    assertPetState(state)
    if (!state.motion?.active)
      return state
    await delay(120)
  }

  throw new Error(`timed out waiting for Lexora Buddy pet motion to finish: ${label}`)
}

function readDurationMs(value, fallback) {
  if (value === undefined)
    return fallback
  const durationMs = Number(value)
  if (!Number.isFinite(durationMs) || durationMs < 0)
    throw new Error(`invalid duration: ${value}`)
  return Math.round(durationMs)
}

function delay(ms) {
  return new Promise(resolve => setTimeout(resolve, ms))
}

function sendControlRequest(request, candidatePath = socketPath) {
  return sendControlMessage(JSON.stringify(request), candidatePath)
}

async function readRuntimeAnimations() {
  if (!runtimeAnimationsPromise) {
    runtimeAnimationsPromise = sendControlRequest({ type: 'capabilities' })
      .then((capabilities) => {
        const runtimeAnimations = Array.isArray(capabilities.animations)
          ? capabilities.animations.filter(animation => typeof animation === 'string')
          : []
        if (runtimeAnimations.length === 0)
          throw new Error('runtime capabilities did not include animations')
        return new Set(runtimeAnimations)
      })
  }

  return runtimeAnimationsPromise
}

async function normalizeAnimation(name) {
  const normalized = String(name || '').trim().toLowerCase().replaceAll('-', '_')
  const animation = animationAliases.get(normalized) || normalized
  const animations = await readRuntimeAnimations()
  if (!animations.has(animation))
    throw new Error(`unknown animation: ${name}`)
  return animation
}

async function readOptionalAnimation(value) {
  if (value === undefined)
    return undefined
  return normalizeAnimation(value)
}

function normalizeEdge(value) {
  const edge = String(value || '').trim().toLowerCase()
  switch (edge) {
    case 'left':
    case 'right':
    case 'top':
    case 'bottom':
      return edge
    case 'up':
      return 'top'
    case 'down':
      return 'bottom'
    default:
      throw new Error(`unsupported edge: ${value}`)
  }
}

function sendControlMessage(message, candidatePath = socketPath) {
  return new Promise((resolve, reject) => {
    let response = ''
    const client = net.createConnection({ path: candidatePath })
    const timer = setTimeout(() => {
      client.destroy()
      reject(new Error(`timed out connecting to Lexora Buddy pet socket: ${candidatePath}`))
    }, connectTimeoutMs)

    client.on('connect', () => {
      client.end(`${message}\n`)
    })
    client.on('data', (chunk) => {
      response += chunk.toString('utf8')
    })
    client.on('error', (error) => {
      clearTimeout(timer)
      reject(new Error(`cannot control Lexora Buddy pet: ${error.message}`))
    })
    client.on('end', () => {
      clearTimeout(timer)
      const line = response.trim().split('\n').find(Boolean)
      if (!line) {
        reject(new Error('Lexora Buddy pet socket closed without acknowledgement'))
        return
      }
      const value = JSON.parse(line)
      if (!value.ok) {
        reject(new Error(value.error || 'Lexora Buddy pet rejected the control message'))
        return
      }
      resolve(value)
    })
  })
}

async function diagnose() {
  const activeWindow = detectActiveWindow()
  const binaries = detectBuddyBinaries()
  const sidecars = detectNativePetSidecars()
  const socketProbe = await probePetSocket(socketPath)
  return {
    ok: true,
    platform: process.platform,
    desktop: {
      xdgCurrentDesktop: process.env.XDG_CURRENT_DESKTOP || null,
      xdgSessionDesktop: process.env.XDG_SESSION_DESKTOP || null,
      waylandDisplay: process.env.WAYLAND_DISPLAY || null,
      display: process.env.DISPLAY || null,
    },
    commands: {
      qdbus6: commandPath('qdbus6'),
      gdbus: commandPath('gdbus'),
      kdotool: commandPath('kdotool'),
      xdotool: commandPath('xdotool'),
    },
    installation: {
      packageNames: {
        deb: 'lexora-buddy',
        pacman: 'lexora-buddy-bin',
      },
      packages: detectInstalledPackages(),
      binaries,
      launchable: binaries.some(binary => Boolean(binary.path)),
    },
    socket: {
      env: socketEnvName,
      path: socketPath,
      exists: fs.existsSync(socketPath),
      connectable: socketProbe.connectable,
      responsive: socketProbe.responsive,
      responseError: socketProbe.responseError,
    },
    runtime: {
      sidecarCount: sidecars.length,
      sidecars,
    },
    activeWindow,
  }
}

async function probePetSocket(candidatePath) {
  const connectable = await canConnectSocket(candidatePath)
  const controlProtocol = await probePetControlProtocol(connectable, candidatePath)
  return {
    connectable,
    responsive: controlProtocol.responsive,
    responseError: controlProtocol.responseError,
  }
}

async function probePetControlProtocol(connectable, candidatePath = socketPath) {
  if (!connectable) {
    return {
      responsive: false,
      responseError: null,
    }
  }

  try {
    await readPetState(candidatePath)
    return {
      responsive: true,
      responseError: null,
    }
  }
  catch (error) {
    return {
      responsive: false,
      responseError: error instanceof Error ? error.message : String(error),
    }
  }
}

async function launchBuddyPet() {
  const socketProbe = await probePetSocket(socketPath)
  if (socketProbe.responsive) {
    const sidecars = usesCustomSocketPath ? [] : detectNativePetSidecars()
    return {
      ok: true,
      reused: true,
      pid: sidecars[0]?.pid ?? null,
      sidecarCount: sidecars.length,
      socketPath,
      message: 'Lexora Buddy pet is already running',
    }
  }

  const existingSidecars = usesCustomSocketPath ? [] : detectNativePetSidecars()
  if (existingSidecars.length > 0) {
    const ready = await waitForResponsiveSocket(socketPath, launchWaitMs)
    if (ready.responsive) {
      return {
        ok: true,
        reused: true,
        pid: existingSidecars[0].pid,
        sidecarCount: existingSidecars.length,
        socketPath,
        message: 'Lexora Buddy pet sidecar is already running',
      }
    }

    const detail = ready.responseError ? ` Last response error: ${ready.responseError}.` : ''
    throw new Error(`Lexora Buddy pet sidecar is already running without a responsive control socket (${formatSidecarPids(existingSidecars)}).${detail} Close stale native-pet sidecars before launching another one.`)
  }

  const binary = detectBuddyBinaries().find(candidate => candidate.path)
  if (!binary)
    throw new Error('lexora-buddy is not installed or not in PATH')

  const child = spawn(binary.name, [], {
    detached: true,
    stdio: 'ignore',
  })
  child.unref()

  const ready = await waitForResponsiveSocket(socketPath, launchWaitMs)
  return {
    ok: ready.responsive,
    binary: binary.name,
    pid: child.pid,
    responseError: ready.responseError,
    socketPath,
    message: ready.responsive
      ? 'Lexora Buddy pet is ready'
      : 'Lexora Buddy was launched, but the pet control socket is not responsive yet',
  }
}

function detectBuddyBinaries() {
  return buddyBinaries.map((name) => {
    const resolvedPath = commandPath(name)
    return {
      name,
      path: resolvedPath,
      sha256: resolvedPath ? sha256File(resolvedPath) : null,
    }
  })
}

function detectInstalledPackages() {
  return {
    pacman: detectPacmanPackage('lexora-buddy-bin'),
  }
}

function detectPacmanPackage(packageName) {
  if (!commandPath('pacman'))
    return null

  try {
    return parsePacmanPackageInfo(execFileSync('pacman', ['-Qi', packageName], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }), packageName)
  }
  catch {
    return null
  }
}

function parsePacmanPackageInfo(output, fallbackName) {
  const fields = new Map()
  for (const line of output.split('\n')) {
    const separatorIndex = line.indexOf(':')
    if (separatorIndex < 0)
      continue

    const key = line.slice(0, separatorIndex).trim()
    const value = line.slice(separatorIndex + 1).trim()
    if (key)
      fields.set(key, value || null)
  }

  return {
    name: fields.get('Name') || fallbackName,
    version: fields.get('Version') || null,
    installDate: fields.get('Install Date') || null,
  }
}

function sha256File(filePath) {
  try {
    return createHash('sha256').update(fs.readFileSync(filePath)).digest('hex')
  }
  catch {
    return null
  }
}

function detectNativePetSidecars() {
  if (process.platform === 'win32')
    return []

  let output = ''
  try {
    output = execFileSync('ps', ['-eo', 'pid=,ppid=,stat=,command='], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    })
  }
  catch {
    return []
  }

  return output
    .split('\n')
    .map(line => parseProcessLine(line))
    .filter(Boolean)
    .filter(processInfo => processInfo.command.includes('lexora-buddy')
      && processInfo.command.includes('--native-pet'))
}

function parseProcessLine(line) {
  let rest = line
  const pid = readProcessToken(rest)
  if (!pid)
    return null
  rest = pid.rest

  const ppid = readProcessToken(rest)
  if (!ppid)
    return null
  rest = ppid.rest

  const stat = readProcessToken(rest)
  if (!stat)
    return null
  rest = stat.rest

  const command = rest.trim()
  if (!command)
    return null

  return {
    pid: Number(pid.token),
    ppid: Number(ppid.token),
    stat: stat.token,
    command,
  }
}

function readProcessToken(value) {
  let start = 0
  while (start < value.length && isProcessWhitespace(value[start]))
    start += 1

  let end = start
  while (end < value.length && !isProcessWhitespace(value[end]))
    end += 1

  if (end === start)
    return null

  return {
    token: value.slice(start, end),
    rest: value.slice(end),
  }
}

function isProcessWhitespace(value) {
  return value === ' ' || value === '\t'
}

function formatSidecarPids(sidecars) {
  return sidecars.map(sidecar => `pid=${sidecar.pid}`).join(', ')
}

async function waitForResponsiveSocket(candidatePath, timeoutMs) {
  const startedAt = Date.now()
  let lastProbe = {
    connectable: false,
    responsive: false,
    responseError: null,
  }
  while (Date.now() - startedAt < timeoutMs) {
    lastProbe = await probePetSocket(candidatePath)
    if (lastProbe.responsive)
      return lastProbe
    await delay(200)
  }

  return lastProbe
}

function canConnectSocket(candidatePath) {
  return new Promise((resolve) => {
    const client = net.createConnection({ path: candidatePath })
    const timer = setTimeout(() => {
      client.destroy()
      resolve(false)
    }, 500)
    client.on('connect', () => {
      clearTimeout(timer)
      client.end()
      resolve(true)
    })
    client.on('error', () => {
      clearTimeout(timer)
      resolve(false)
    })
  })
}

function detectActiveWindow() {
  if (commandPath('xdotool')) {
    try {
      const output = execFileSync('sh', ['-lc', 'xdotool getactivewindow getwindowgeometry --shell'], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      })
      const values = Object.fromEntries(
        output
          .trim()
          .split('\n')
          .map(line => line.split('='))
          .filter(parts => parts.length === 2),
      )
      return {
        ok: true,
        source: 'xdotool',
        x: Number(values.X),
        y: Number(values.Y),
        width: Number(values.WIDTH),
        height: Number(values.HEIGHT),
      }
    }
    catch (error) {
      return unavailableWindow(`xdotool failed: ${error.message}`)
    }
  }

  if (commandPath('qdbus6') && isKdeDesktop()) {
    return detectKwinActiveWindow()
  }

  return unavailableWindow('no supported active window detector found')
}

function detectKwinActiveWindow() {
  const pluginName = `lexora-buddy-active-window-${process.pid}-${Date.now()}`
  const outputToken = `${kwinActiveWindowOutputPrefix}${pluginName}:`
  const script = createKwinActiveWindowScript(outputToken)

  if (!runTemporaryKwinScript(pluginName, script))
    return unavailableWindow('qdbus6 failed to query KWin active window')

  const rect = pollKwinActiveWindowJournal(outputToken)
  if (rect === undefined)
    return unavailableWindow('KWin active window query did not return geometry')
  if (rect === null)
    return unavailableWindow('KWin active window is not a usable target')

  return {
    ok: true,
    source: 'kwin-journal',
    ...rect,
  }
}

function runTemporaryKwinScript(pluginName, script) {
  const scriptDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lexora-buddy-kwin-'))
  const scriptPath = path.join(scriptDir, `${pluginName}.js`)
  try {
    fs.writeFileSync(scriptPath, script)
    runQdbus6([
      'org.kde.KWin',
      '/Scripting',
      'org.kde.kwin.Scripting.unloadScript',
      pluginName,
    ], true)
    runQdbus6([
      'org.kde.KWin',
      '/Scripting',
      'org.kde.kwin.Scripting.loadScript',
      scriptPath,
      pluginName,
    ])
    runQdbus6([
      'org.kde.KWin',
      '/Scripting',
      'org.kde.kwin.Scripting.start',
    ])
    return true
  }
  catch {
    return false
  }
  finally {
    try {
      runQdbus6([
        'org.kde.KWin',
        '/Scripting',
        'org.kde.kwin.Scripting.unloadScript',
        pluginName,
      ], true)
    }
    catch {}
    fs.rmSync(scriptDir, { force: true, recursive: true })
  }
}

function runQdbus6(args, ignoreErrors = false) {
  try {
    return execFileSync('qdbus6', args, {
      encoding: 'utf8',
      env: {
        ...process.env,
        ...sessionBusEnv(),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
  }
  catch (error) {
    if (ignoreErrors)
      return ''
    throw error
  }
}

function sessionBusEnv() {
  if (process.env.DBUS_SESSION_BUS_ADDRESS)
    return {}
  if (!process.env.XDG_RUNTIME_DIR)
    return {}
  return {
    DBUS_SESSION_BUS_ADDRESS: `unix:path=${process.env.XDG_RUNTIME_DIR}/bus`,
  }
}

function pollKwinActiveWindowJournal(outputToken) {
  const deadline = Date.now() + kwinActiveWindowQueryTimeoutMs
  while (Date.now() <= deadline) {
    const rect = readKwinActiveWindowJournal(outputToken)
    if (rect !== undefined)
      return rect
    sleepSync(kwinActiveWindowPollIntervalMs)
  }
  return undefined
}

function readKwinActiveWindowJournal(outputToken) {
  try {
    const output = execFileSync('journalctl', [
      '--user',
      '-u',
      'plasma-kwin_wayland.service',
      '-n',
      '80',
      '--no-pager',
    ], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    })
    return parseKwinActiveWindowRectOutput(output, outputToken)
  }
  catch {
    return undefined
  }
}

function parseKwinActiveWindowRectOutput(output, outputToken) {
  return output
    .split('\n')
    .reverse()
    .map(line => parseKwinActiveWindowRectOutputLine(line, outputToken))
    .find(value => value !== undefined)
}

function parseKwinActiveWindowRectOutputLine(line, outputToken) {
  const tokenIndex = line.indexOf(outputToken)
  if (tokenIndex < 0)
    return undefined

  const payload = line.slice(tokenIndex + outputToken.length)
  if (kwinActiveWindowPayloadIsNull(payload))
    return null

  const x = extractKwinOutputNumber(payload, 'x')
  const y = extractKwinOutputNumber(payload, 'y')
  const width = extractKwinOutputNumber(payload, 'width')
  const height = extractKwinOutputNumber(payload, 'height')
  if (!Number.isFinite(x) || !Number.isFinite(y) || width <= 0 || height <= 0)
    return undefined

  return { x, y, width, height }
}

function kwinActiveWindowPayloadIsNull(payload) {
  return payload
    .replace(/^[\s"'\\(,]+/, '')
    .startsWith('null')
}

function extractKwinOutputNumber(payload, key) {
  for (const marker of [`"${key}":`, `\\"${key}\\":`, `${key}:`]) {
    const markerIndex = payload.indexOf(marker)
    if (markerIndex >= 0)
      return parseOutputNumber(payload.slice(markerIndex + marker.length))
  }
  return undefined
}

function parseOutputNumber(value) {
  const match = value.match(/-?\d+(?:\.\d+)?/)
  if (!match)
    return undefined
  const number = Number(match[0])
  if (!Number.isFinite(number))
    return undefined
  return Math.round(number)
}

function sleepSync(ms) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)
}

function createKwinActiveWindowScript(outputToken) {
  return `
(function () {
    const token = "${outputToken}";
    const selfWindowMarkers = ["lexora-buddy", "lexora buddy"];

    function normalized(value) {
        if (value === undefined || value === null)
            return "";
        return String(value).toLowerCase();
    }

    function isSelfWindow(window) {
        const identity = [
            window.resourceClass,
            window.resourceName,
            window.desktopFileName,
            window.windowRole
        ].map(normalized).join(" ");
        return selfWindowMarkers.some((marker) => identity.includes(marker));
    }

    function isUsableTargetWindow(window) {
        if (!window)
            return false;
        if (window.deleted || window.minimized)
            return false;
        if (window.normalWindow === false)
            return false;
        if (window.onCurrentDesktop === false || window.onCurrentActivity === false)
            return false;
        if (window.dock || window.desktopWindow || window.splash || window.toolbar || window.menu)
            return false;
        return !isSelfWindow(window);
    }

    const activeWindow = workspace.activeWindow;
    if (!isUsableTargetWindow(activeWindow)) {
        print(token + "null");
        return;
    }

    const geometry = activeWindow.frameGeometry;
    if (!geometry || geometry.width <= 0 || geometry.height <= 0) {
        print(token + "null");
        return;
    }

    print(token + JSON.stringify({
        x: Math.round(geometry.x),
        y: Math.round(geometry.y),
        width: Math.round(geometry.width),
        height: Math.round(geometry.height)
    }));
})();
`
}

function unavailableWindow(reason) {
  return { ok: false, source: null, reason }
}

function isKdeDesktop() {
  return [
    process.env.XDG_CURRENT_DESKTOP,
    process.env.XDG_SESSION_DESKTOP,
    process.env.DESKTOP_SESSION,
  ].some(value => String(value || '').toLowerCase().includes('kde')
    || String(value || '').toLowerCase().includes('plasma'))
}

function commandPath(name) {
  try {
    return execFileSync('sh', ['-lc', `command -v ${shellQuote(name)}`], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim() || null
  }
  catch {
    return null
  }
}

function parseOptions(args) {
  const options = {}
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (!arg.startsWith('--'))
      continue
    const key = arg.slice(2)
    const next = args[index + 1]
    if (next === undefined || next.startsWith('--')) {
      options[key] = true
      continue
    }
    options[key] = next
    index += 1
  }
  return options
}

function readRequiredNumberOption(options, name) {
  const value = Number(options[name])
  if (!Number.isFinite(value))
    throw new Error(`missing numeric --${name}`)
  return value
}

function requiredArg(args, label) {
  const value = args.find(arg => !arg.startsWith('--'))
  if (!value)
    throw new Error(`missing ${label}`)
  return value
}

function shellQuote(value) {
  return `'${String(value).replaceAll('\'', '\'\\\'\'')}'`
}

function printJson(value) {
  console.log(JSON.stringify(value, null, 2))
}

function printHelp() {
  console.log(`Lexora Buddy pet control

Usage:
  node scripts/lexora-buddy-pet.mjs diagnose
  node scripts/lexora-buddy-pet.mjs state
  node scripts/lexora-buddy-pet.mjs capabilities
  node scripts/lexora-buddy-pet.mjs animation celebrate
  node scripts/lexora-buddy-pet.mjs move center
  node scripts/lexora-buddy-pet.mjs move home --after sleep
  node scripts/lexora-buddy-pet.mjs walk-window-left --after celebrate
  node scripts/lexora-buddy-pet.mjs walk-window-right --after explain
  node scripts/lexora-buddy-pet.mjs walk-to-edge top --after curious
  node scripts/lexora-buddy-pet.mjs walk-to-edge bottom --after celebrate
  node scripts/lexora-buddy-pet.mjs walk-to --x 120 --y 640 --after curious
  node scripts/lexora-buddy-pet.mjs perform center-cast-return-sleep --animation cast --duration-ms 2000
  node scripts/lexora-buddy-pet.mjs sequence --json '{"steps":[{"type":"snapshot","name":"original"},{"type":"move","target":"center"},{"type":"animation","animation":"celebrate","durationMs":2000},{"type":"move","target":{"kind":"snapshot","name":"original"},"after":"sleep"}]}'
  node scripts/lexora-buddy-pet.mjs sidecars
`)
}

await main()
