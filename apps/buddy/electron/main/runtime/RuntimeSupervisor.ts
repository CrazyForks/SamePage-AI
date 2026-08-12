import type { Readable, Writable } from 'node:stream'
import type { RuntimeNotification, RuntimeRequestOptions } from './RuntimeRpcClient'
import nodeProcess from 'node:process'
import { RuntimeRpcClient, RuntimeUnavailableError } from './RuntimeRpcClient'

export interface RuntimeChildProcess {
  readonly pid?: number
  readonly stdin: Writable
  readonly stdout: Readable
  readonly stderr: Readable
  kill: (signal?: NodeJS.Signals | number) => boolean
  once: (
    ((event: 'error', listener: (error: Error) => void) => RuntimeChildProcess)
    & ((event: 'exit', listener: (code: number | null, signal: NodeJS.Signals | null) => void) => RuntimeChildProcess)
  )
}

export type RuntimeSupervisorStatus
  = | 'stopped'
    | 'starting'
    | 'ready'
    | 'restarting'
    | 'offline'
    | 'stopping'

export interface RuntimeSupervisorState {
  status: RuntimeSupervisorStatus
  pid: number | null
  restartAttempt: number
  lastError: string | null
}

export interface RuntimeSupervisorOptions {
  spawnRuntime: () => RuntimeChildProcess
  restartDelaysMs?: number[]
  stableResetMs?: number
  shutdownTimeoutMs?: number
  forceKillTimeoutMs?: number
  readinessTimeoutMs?: number
  diagnosticOutput?: Writable
}

const DEFAULT_RESTART_DELAYS_MS = [500, 1_000, 2_000, 5_000, 10_000]
const DEFAULT_STABLE_RESET_MS = 60_000
const DEFAULT_SHUTDOWN_TIMEOUT_MS = 5_000
const DEFAULT_FORCE_KILL_TIMEOUT_MS = 2_000
const DEFAULT_READINESS_TIMEOUT_MS = 30_000
const RUNTIME_PROTOCOL_VERSION = 2

export class RuntimeSupervisor {
  readonly #spawnRuntime: () => RuntimeChildProcess
  readonly #restartDelaysMs: number[]
  readonly #stableResetMs: number
  readonly #shutdownTimeoutMs: number
  readonly #forceKillTimeoutMs: number
  readonly #readinessTimeoutMs: number
  readonly #diagnosticOutput: Writable
  readonly #stateListeners = new Set<(state: RuntimeSupervisorState) => void>()
  readonly #notificationListeners = new Set<(notification: RuntimeNotification) => void>()
  readonly #exitedProcesses = new WeakSet<RuntimeChildProcess>()
  #state: RuntimeSupervisorState = {
    status: 'stopped',
    pid: null,
    restartAttempt: 0,
    lastError: null,
  }

  #desiredRunning = false
  #generation = 0
  #process: RuntimeChildProcess | null = null
  #client: RuntimeRpcClient | null = null
  #restartTimer: ReturnType<typeof setTimeout> | null = null
  #stableTimer: ReturnType<typeof setTimeout> | null = null
  #readinessTimer: ReturnType<typeof setTimeout> | null = null
  #lifecycleTail: Promise<void> = Promise.resolve()
  #intent = 0

  constructor(options: RuntimeSupervisorOptions) {
    this.#spawnRuntime = options.spawnRuntime
    this.#restartDelaysMs = options.restartDelaysMs ?? DEFAULT_RESTART_DELAYS_MS
    this.#stableResetMs = options.stableResetMs ?? DEFAULT_STABLE_RESET_MS
    this.#shutdownTimeoutMs = options.shutdownTimeoutMs ?? DEFAULT_SHUTDOWN_TIMEOUT_MS
    this.#forceKillTimeoutMs = options.forceKillTimeoutMs ?? DEFAULT_FORCE_KILL_TIMEOUT_MS
    this.#readinessTimeoutMs = options.readinessTimeoutMs ?? DEFAULT_READINESS_TIMEOUT_MS
    this.#diagnosticOutput = options.diagnosticOutput ?? nodeProcess.stderr
  }

  get state(): RuntimeSupervisorState {
    return this.#state
  }

  start(): void {
    if (this.#desiredRunning && this.#state.status !== 'offline')
      return

    const intent = ++this.#intent
    this.#clearTimers()
    this.#desiredRunning = true
    this.#setState({
      status: 'starting',
      pid: null,
      restartAttempt: 0,
      lastError: null,
    })
    if (!this.#process) {
      this.#spawnGeneration()
      return
    }

    void this.#enqueueLifecycle(async () => {
      const stopped = await this.#stopCurrentProcess()
      if (!stopped) {
        if (intent === this.#intent && this.#desiredRunning)
          this.#setTerminationFailure()
        return
      }
      if (intent === this.#intent && this.#desiredRunning && !this.#process)
        this.#spawnGeneration()
    })
  }

  async request(
    method: string,
    params: unknown,
    options?: RuntimeRequestOptions,
  ): Promise<unknown> {
    const client = await this.#waitForReadyClient()
    return client.request(method, params, options)
  }

  onStateChange(listener: (state: RuntimeSupervisorState) => void): () => void {
    this.#stateListeners.add(listener)
    return () => this.#stateListeners.delete(listener)
  }

  onNotification(listener: (notification: RuntimeNotification) => void): () => void {
    this.#notificationListeners.add(listener)
    return () => this.#notificationListeners.delete(listener)
  }

  async stop(): Promise<void> {
    if (!this.#desiredRunning && this.#state.status === 'stopped')
      return

    const intent = ++this.#intent
    this.#desiredRunning = false
    this.#clearTimers()
    this.#setState({ ...this.#state, status: 'stopping' })

    return this.#enqueueLifecycle(async () => {
      const stopped = await this.#stopCurrentProcess()
      if (intent !== this.#intent || this.#desiredRunning)
        return

      if (stopped)
        this.#setStopped()
      else
        this.#setTerminationFailure()
    })
  }

  async restart(): Promise<void> {
    const intent = ++this.#intent
    this.#desiredRunning = true
    this.#clearTimers()
    this.#setState({
      ...this.#state,
      status: 'restarting',
      lastError: null,
    })

    return this.#enqueueLifecycle(async () => {
      const stopped = await this.#stopCurrentProcess()
      if (intent !== this.#intent || !this.#desiredRunning)
        return
      if (!stopped) {
        this.#setTerminationFailure()
        return
      }

      this.#setState({
        status: 'starting',
        pid: null,
        restartAttempt: 0,
        lastError: null,
      })
      this.#spawnGeneration()
    })
  }

  async #stopCurrentProcess(): Promise<boolean> {
    const process = this.#process
    const client = this.#client
    if (!process)
      return true

    this.#generation += 1
    this.#clearReadinessTimer()
    this.#clearStableTimer()
    const exited = waitForProcessExit(process, this.#exitedProcesses.has(process))
    let exitedProcess = false
    if (client) {
      void client.request('runtime.shutdown', {}).catch(() => {})
      exitedProcess = await settleWithin(exited, this.#shutdownTimeoutMs)
    }

    if (!exitedProcess) {
      process.kill('SIGTERM')
      exitedProcess = await settleWithin(exited, this.#forceKillTimeoutMs)
      if (!exitedProcess) {
        process.kill('SIGKILL')
        exitedProcess = await settleWithin(exited, this.#forceKillTimeoutMs)
      }
    }

    client?.close()
    if (exitedProcess && this.#process === process) {
      this.#client = null
      this.#process = null
    }
    return exitedProcess
  }

  #spawnGeneration(): void {
    if (!this.#desiredRunning)
      return

    const generation = ++this.#generation
    let process: RuntimeChildProcess
    try {
      process = this.#spawnRuntime()
    }
    catch (error) {
      this.#handleSpawnFailure(generation, error)
      return
    }

    const client = new RuntimeRpcClient({
      readable: process.stdout,
      writable: process.stdin,
    })
    process.stderr.pipe(this.#diagnosticOutput, { end: false })
    this.#process = process
    this.#client = client
    this.#setState({
      ...this.#state,
      status: 'starting',
      pid: process.pid ?? null,
      lastError: null,
    })
    this.#scheduleReadinessTimeout(generation, process, client)

    client.onNotification((notification) => {
      if (generation !== this.#generation)
        return

      if (notification.method === 'runtime.ready') {
        const protocolVersion = readProtocolVersion(notification.params)
        if (protocolVersion !== RUNTIME_PROTOCOL_VERSION) {
          this.#failGeneration(
            generation,
            process,
            client,
            `Unsupported Lexora Runtime protocol version: ${protocolVersion ?? 'unknown'}`,
          )
          return
        }

        this.#clearReadinessTimer()
        this.#setState({
          ...this.#state,
          status: 'ready',
          pid: process.pid ?? null,
          lastError: null,
        })
        this.#scheduleStableReset(generation)
        return
      }

      for (const listener of this.#notificationListeners)
        listener(notification)
    })
    client.onFatalError((error) => {
      this.#failGeneration(generation, process, client, error.message)
    })

    process.once('exit', (code, signal) => {
      this.#exitedProcesses.add(process)
      this.#handleExit(generation, process, client, code, signal)
    })
    process.once('error', (error) => {
      if (process.pid === undefined)
        this.#exitedProcesses.add(process)
      this.#handleProcessError(generation, process, client, error)
    })
  }

  #handleProcessError(
    generation: number,
    process: RuntimeChildProcess,
    client: RuntimeRpcClient,
    error: Error,
  ): void {
    if (generation !== this.#generation || !this.#desiredRunning)
      return

    this.#failGeneration(
      generation,
      process,
      client,
      error.message || 'Runtime process failed',
      'Lexora Runtime failed to start',
    )
  }

  #handleSpawnFailure(generation: number, error: unknown): void {
    if (generation !== this.#generation || !this.#desiredRunning)
      return

    const diagnostic = error instanceof Error ? error.message : 'Runtime failed to start'
    this.#writeDiagnostic(diagnostic)
    this.#scheduleRestart('Lexora Runtime failed to start')
  }

  #handleExit(
    generation: number,
    process: RuntimeChildProcess,
    client: RuntimeRpcClient,
    code: number | null,
    signal: NodeJS.Signals | null,
  ): void {
    if (generation !== this.#generation)
      return

    this.#generation += 1
    this.#clearReadinessTimer()
    this.#clearStableTimer()
    client.close()
    if (this.#process === process) {
      this.#process = null
      this.#client = null
    }

    if (!this.#desiredRunning) {
      this.#setStopped()
      return
    }

    const reason = signal
      ? `Runtime exited from ${signal}`
      : `Runtime exited with code ${code ?? 'unknown'}`
    this.#writeDiagnostic(reason)
    this.#scheduleRestart('Lexora Runtime stopped unexpectedly')
  }

  #scheduleRestart(error: string): void {
    if (!this.#desiredRunning)
      return

    const attempt = this.#state.restartAttempt
    const delay = this.#restartDelaysMs[attempt]
    if (delay === undefined) {
      this.#setState({
        status: 'offline',
        pid: null,
        restartAttempt: attempt,
        lastError: error,
      })
      return
    }

    const nextAttempt = attempt + 1
    this.#setState({
      status: 'restarting',
      pid: null,
      restartAttempt: nextAttempt,
      lastError: error,
    })
    this.#restartTimer = setTimeout(() => {
      this.#restartTimer = null
      this.#spawnGeneration()
    }, delay)
  }

  #scheduleStableReset(generation: number): void {
    this.#clearStableTimer()
    this.#stableTimer = setTimeout(() => {
      this.#stableTimer = null
      if (generation !== this.#generation || this.#state.status !== 'ready')
        return

      this.#setState({
        ...this.#state,
        restartAttempt: 0,
      })
    }, this.#stableResetMs)
  }

  #scheduleReadinessTimeout(
    generation: number,
    process: RuntimeChildProcess,
    client: RuntimeRpcClient,
  ): void {
    this.#clearReadinessTimer()
    this.#readinessTimer = setTimeout(() => {
      this.#readinessTimer = null
      this.#failGeneration(
        generation,
        process,
        client,
        'Lexora Runtime did not become ready in time',
      )
    }, this.#readinessTimeoutMs)
  }

  #failGeneration(
    generation: number,
    process: RuntimeChildProcess,
    client: RuntimeRpcClient,
    error: string,
    publicError = toPublicRuntimeError(error),
  ): void {
    if (generation !== this.#generation || !this.#desiredRunning)
      return

    this.#generation += 1
    this.#clearReadinessTimer()
    this.#clearStableTimer()
    client.close()
    if (this.#process === process)
      this.#client = null
    this.#writeDiagnostic(error)
    this.#setState({
      status: 'restarting',
      pid: process.pid ?? null,
      restartAttempt: this.#state.restartAttempt,
      lastError: publicError,
    })

    const intent = this.#intent
    void this.#enqueueLifecycle(async () => {
      const terminated = await terminateProcess(
        process,
        this.#forceKillTimeoutMs,
        this.#exitedProcesses.has(process),
      )
      if (terminated && this.#process === process)
        this.#process = null
      if (intent !== this.#intent || !this.#desiredRunning)
        return

      if (!terminated) {
        this.#setTerminationFailure()
        return
      }
      this.#scheduleRestart(publicError)
    })
  }

  #writeDiagnostic(message: string): void {
    this.#diagnosticOutput.write(`[Lexora Runtime] ${message}\n`)
  }

  #setStopped(): void {
    this.#setState({
      status: 'stopped',
      pid: null,
      restartAttempt: 0,
      lastError: null,
    })
  }

  #setTerminationFailure(): void {
    this.#setState({
      status: 'offline',
      pid: this.#process?.pid ?? null,
      restartAttempt: this.#state.restartAttempt,
      lastError: 'Lexora Runtime could not be terminated',
    })
  }

  #setState(state: RuntimeSupervisorState): void {
    this.#state = Object.freeze({ ...state })
    for (const listener of this.#stateListeners)
      listener(this.#state)
  }

  #waitForReadyClient(): Promise<RuntimeRpcClient> {
    if (this.#state.status === 'ready' && this.#client)
      return Promise.resolve(this.#client)

    if (!this.#desiredRunning || this.#state.status === 'offline' || this.#state.status === 'stopping')
      return Promise.reject(new RuntimeUnavailableError())

    return new Promise((resolve, reject) => {
      let stopListening = () => {}
      const timeout = setTimeout(() => {
        stopListening()
        reject(new RuntimeUnavailableError('Lexora Runtime did not become ready in time'))
      }, this.#readinessTimeoutMs)
      stopListening = this.onStateChange((state) => {
        if (state.status === 'ready' && this.#client) {
          clearTimeout(timeout)
          stopListening()
          resolve(this.#client)
          return
        }

        if (state.status === 'offline' || state.status === 'stopped' || state.status === 'stopping') {
          clearTimeout(timeout)
          stopListening()
          reject(new RuntimeUnavailableError())
        }
      })
    })
  }

  #clearTimers(): void {
    if (this.#restartTimer) {
      clearTimeout(this.#restartTimer)
      this.#restartTimer = null
    }
    this.#clearReadinessTimer()
    this.#clearStableTimer()
  }

  #clearReadinessTimer(): void {
    if (!this.#readinessTimer)
      return

    clearTimeout(this.#readinessTimer)
    this.#readinessTimer = null
  }

  #clearStableTimer(): void {
    if (!this.#stableTimer)
      return

    clearTimeout(this.#stableTimer)
    this.#stableTimer = null
  }

  #enqueueLifecycle(operation: () => Promise<void>): Promise<void> {
    const next = this.#lifecycleTail.then(operation, operation)
    this.#lifecycleTail = next.catch(() => {})
    return next
  }
}

function waitForProcessExit(process: RuntimeChildProcess, alreadyExited = false): Promise<void> {
  if (alreadyExited)
    return Promise.resolve()

  return new Promise((resolve) => {
    process.once('exit', () => resolve())
  })
}

function settleWithin(promise: Promise<unknown>, timeoutMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    const timeout = setTimeout(resolve, timeoutMs, false)
    promise.then(
      () => {
        clearTimeout(timeout)
        resolve(true)
      },
      () => {
        clearTimeout(timeout)
        resolve(false)
      },
    )
  })
}

function readProtocolVersion(params: unknown): number | null {
  if (typeof params !== 'object' || params === null || Array.isArray(params))
    return null

  const version = Reflect.get(params, 'protocolVersion')
  return typeof version === 'number' && Number.isInteger(version) ? version : null
}

async function terminateProcess(
  process: RuntimeChildProcess,
  forceKillTimeoutMs: number,
  alreadyExited = false,
): Promise<boolean> {
  const exited = waitForProcessExit(process, alreadyExited)
  if (alreadyExited)
    return true

  process.kill('SIGTERM')
  if (await settleWithin(exited, forceKillTimeoutMs))
    return true

  process.kill('SIGKILL')
  return settleWithin(exited, forceKillTimeoutMs)
}

function toPublicRuntimeError(error: string): string {
  if (error === 'Lexora Runtime did not become ready in time')
    return error
  if (error.startsWith('Unsupported Lexora Runtime protocol version:'))
    return 'Lexora Runtime protocol is incompatible'
  return 'Lexora Runtime protocol failed'
}
