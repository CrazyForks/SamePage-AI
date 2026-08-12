import type { Readable, Writable } from 'node:stream'
import { Buffer } from 'node:buffer'
import { randomUUID } from 'node:crypto'
import { StringDecoder } from 'node:string_decoder'

const DEFAULT_MAX_LINE_BYTES = 8 * 1024 * 1024
const DEFAULT_REQUEST_TIMEOUT_MS = 30_000

export interface RuntimeNotification {
  method: string
  params: unknown
}

export interface RuntimeRpcClientOptions {
  readable: Readable
  writable: Writable
  maxLineBytes?: number
  requestTimeoutMs?: number
}

export interface RuntimeRequestOptions {
  timeoutMs?: number
}

interface PendingRequest {
  reject: (error: Error) => void
  resolve: (result: unknown) => void
  timeout: ReturnType<typeof setTimeout>
}

export class RuntimeProtocolError extends Error {
  readonly code = 'RUNTIME_PROTOCOL_ERROR'

  constructor(message: string, options?: ErrorOptions) {
    super(message, options)
    this.name = 'RuntimeProtocolError'
  }
}

export class RuntimeUnavailableError extends Error {
  readonly code = 'RUNTIME_UNAVAILABLE'

  constructor(message = 'Lexora Runtime is unavailable', options?: ErrorOptions) {
    super(message, options)
    this.name = 'RuntimeUnavailableError'
  }
}

export class RuntimeRequestError extends Error {
  readonly code: number
  readonly data: unknown

  constructor(error: { code: number, data?: unknown, message: string }) {
    super(error.message)
    this.name = 'RuntimeRequestError'
    this.code = error.code
    this.data = error.data
  }
}

export class RuntimeRpcClient {
  readonly #readable: Readable
  readonly #writable: Writable
  readonly #decoder = new StringDecoder('utf8')
  readonly #maxLineBytes: number
  readonly #requestTimeoutMs: number
  readonly #pendingRequests = new Map<string, PendingRequest>()
  readonly #notificationListeners = new Set<(notification: RuntimeNotification) => void>()
  readonly #fatalErrorListeners = new Set<(error: Error) => void>()
  #buffer = ''
  #closed = false

  constructor(options: RuntimeRpcClientOptions) {
    this.#readable = options.readable
    this.#writable = options.writable
    this.#maxLineBytes = options.maxLineBytes ?? DEFAULT_MAX_LINE_BYTES
    this.#requestTimeoutMs = options.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS

    this.#readable.on('data', this.#handleData)
    this.#readable.once('end', this.#handleEnd)
    this.#readable.once('error', this.#handleStreamError)
    this.#writable.once('error', this.#handleStreamError)
  }

  request(method: string, params: unknown, options: RuntimeRequestOptions = {}): Promise<unknown> {
    if (this.#closed)
      return Promise.reject(new RuntimeUnavailableError())

    const id = randomUUID()
    const payload = JSON.stringify({
      jsonrpc: '2.0',
      id,
      method,
      params,
    })
    if (Buffer.byteLength(payload) > this.#maxLineBytes)
      return Promise.reject(new RuntimeProtocolError('Runtime request exceeds the size limit'))

    const encoded = `${payload}\n`

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        const pending = this.#pendingRequests.get(id)
        if (!pending)
          return

        this.#pendingRequests.delete(id)
        pending.reject(new RuntimeUnavailableError(`Runtime request timed out: ${method}`))
      }, options.timeoutMs ?? this.#requestTimeoutMs)

      this.#pendingRequests.set(id, { reject, resolve, timeout })
      try {
        this.#writable.write(encoded, (error) => {
          if (error)
            this.#fail(new RuntimeUnavailableError('Failed to write to Lexora Runtime', { cause: error }))
        })
      }
      catch (error) {
        this.#fail(new RuntimeUnavailableError('Failed to write to Lexora Runtime', { cause: error }))
      }
    })
  }

  onNotification(listener: (notification: RuntimeNotification) => void): () => void {
    this.#notificationListeners.add(listener)
    return () => this.#notificationListeners.delete(listener)
  }

  onFatalError(listener: (error: Error) => void): () => void {
    this.#fatalErrorListeners.add(listener)
    return () => this.#fatalErrorListeners.delete(listener)
  }

  close(): void {
    this.#fail(new RuntimeUnavailableError('Lexora Runtime connection closed'))
  }

  readonly #handleData = (chunk: Buffer | string): void => {
    if (this.#closed)
      return

    this.#buffer += typeof chunk === 'string' ? chunk : this.#decoder.write(chunk)
    this.#drainLines()
  }

  readonly #handleEnd = (): void => {
    if (this.#closed)
      return

    this.#buffer += this.#decoder.end()
    if (this.#buffer.trim()) {
      this.#fail(new RuntimeProtocolError('Runtime output ended with an incomplete message'))
      return
    }

    this.#fail(new RuntimeUnavailableError('Lexora Runtime disconnected'))
  }

  readonly #handleStreamError = (error: Error): void => {
    this.#fail(new RuntimeUnavailableError('Lexora Runtime stream failed', { cause: error }))
  }

  #drainLines(): void {
    while (!this.#closed) {
      const newlineIndex = this.#buffer.indexOf('\n')
      if (newlineIndex < 0) {
        if (Buffer.byteLength(this.#buffer) > this.#maxLineBytes)
          this.#fail(new RuntimeProtocolError('Runtime protocol line exceeds the size limit'))

        return
      }

      const line = this.#buffer.slice(0, newlineIndex)
      this.#buffer = this.#buffer.slice(newlineIndex + 1)
      if (!line.trim())
        continue

      if (Buffer.byteLength(line) > this.#maxLineBytes) {
        this.#fail(new RuntimeProtocolError('Runtime protocol line exceeds the size limit'))
        return
      }

      this.#handleLine(line)
    }
  }

  #handleLine(line: string): void {
    let message: unknown
    try {
      message = JSON.parse(line)
    }
    catch (error) {
      this.#fail(new RuntimeProtocolError('Runtime emitted malformed JSON', { cause: error }))
      return
    }

    if (!isRecord(message) || message.jsonrpc !== '2.0') {
      this.#fail(new RuntimeProtocolError('Runtime emitted an invalid JSON-RPC envelope'))
      return
    }

    if (typeof message.method === 'string' && !('id' in message)) {
      const notification = {
        method: message.method,
        params: message.params,
      }
      for (const listener of this.#notificationListeners)
        listener(notification)
      return
    }

    if (typeof message.id !== 'string') {
      this.#fail(new RuntimeProtocolError('Runtime response is missing a string id'))
      return
    }

    const hasResult = 'result' in message
    const hasError = 'error' in message
    if (hasResult === hasError || (hasError && !isRuntimeError(message.error))) {
      this.#fail(new RuntimeProtocolError('Runtime emitted an invalid JSON-RPC response'))
      return
    }

    const pending = this.#pendingRequests.get(message.id)
    if (!pending)
      return

    clearTimeout(pending.timeout)
    this.#pendingRequests.delete(message.id)

    if (hasResult) {
      pending.resolve(message.result)
      return
    }

    pending.reject(new RuntimeRequestError(message.error as { code: number, data?: unknown, message: string }))
  }

  #fail(error: Error): void {
    if (this.#closed)
      return

    this.#closed = true
    this.#readable.off('data', this.#handleData)
    this.#readable.off('end', this.#handleEnd)
    this.#readable.off('error', this.#handleStreamError)
    this.#writable.off('error', this.#handleStreamError)

    for (const pending of this.#pendingRequests.values()) {
      clearTimeout(pending.timeout)
      pending.reject(error)
    }
    this.#pendingRequests.clear()

    for (const listener of this.#fatalErrorListeners)
      listener(error)
    this.#fatalErrorListeners.clear()
    this.#notificationListeners.clear()
  }
}

function isRuntimeError(value: unknown): value is { code: number, data?: unknown, message: string } {
  return isRecord(value)
    && typeof value.code === 'number'
    && typeof value.message === 'string'
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
