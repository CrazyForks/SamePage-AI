import type { ZodError } from 'zod'
import type { LexoraConfig, LexoraConfigPatch } from '../../shared/desktopApi'
import { randomUUID } from 'node:crypto'
import { chmod, mkdir, open, readFile, rename, rm } from 'node:fs/promises'
import { dirname } from 'node:path'
import process from 'node:process'
import { parse, stringify } from 'smol-toml'
import { z } from 'zod'

const desktopConfigSchema = z.object({
  language: z.enum(['zh-CN', 'en-US']).default('zh-CN'),
  launch_at_login: z.boolean().default(false),
  theme: z.enum(['system', 'light', 'dark']).default('system'),
}).passthrough().default({
  language: 'zh-CN',
  launch_at_login: false,
  theme: 'system',
})

const codexConfigSchema = z.object({
  default_model: z.string().default(''),
  reasoning_effort: z.string().trim().min(1).default('medium'),
}).passthrough().default({
  default_model: '',
  reasoning_effort: 'medium',
})

const lexoraConfigFileSchema = z.object({
  agent: z.object({
    codex: codexConfigSchema,
  }).passthrough().default({
    codex: {
      default_model: '',
      reasoning_effort: 'medium',
    },
  }),
  desktop: desktopConfigSchema,
}).passthrough()

export class LexoraConfigError extends Error {
  readonly code = 'INVALID_CONFIG'

  constructor(message: string, options?: ErrorOptions) {
    super(message, options)
    this.name = 'LexoraConfigError'
  }
}

export class LexoraConfigStore {
  readonly #configPath: string
  #writeQueue: Promise<void> = Promise.resolve()

  constructor(options: { configPath: string }) {
    this.#configPath = options.configPath
  }

  async read(): Promise<LexoraConfig> {
    return decodeConfig(await this.#readFile())
  }

  update(patch: LexoraConfigPatch): Promise<LexoraConfig> {
    const operation = this.#writeQueue.then(async () => {
      const file = await this.#readFile()
      const next = mergeConfig(decodeConfig(file), patch)
      await this.#write(mergeConfigFile(file, next))
      return next
    })

    this.#writeQueue = operation.then(() => undefined, () => undefined)
    return operation
  }

  async #readFile(): Promise<unknown> {
    let content: string

    try {
      content = await readFile(this.#configPath, 'utf8')
    }
    catch (error) {
      if (isNodeError(error) && error.code === 'ENOENT')
        return {}

      throw error
    }

    try {
      return content.trim() ? parse(content) : {}
    }
    catch (error) {
      throw createConfigError(error)
    }
  }

  async #write(config: Record<string, unknown>): Promise<void> {
    const parent = dirname(this.#configPath)
    const temporaryPath = `${this.#configPath}.${process.pid}.${randomUUID()}.tmp`
    const content = stringify(config)

    await mkdir(parent, { mode: 0o700, recursive: true })

    try {
      const handle = await open(temporaryPath, 'wx', 0o600)
      try {
        await handle.writeFile(content, 'utf8')
        await handle.sync()
      }
      finally {
        await handle.close()
      }

      await rename(temporaryPath, this.#configPath)
      await chmod(this.#configPath, 0o600)
    }
    finally {
      await rm(temporaryPath, { force: true })
    }
  }
}

function decodeConfig(value: unknown): LexoraConfig {
  let config: z.infer<typeof lexoraConfigFileSchema>

  try {
    config = lexoraConfigFileSchema.parse(value)
  }
  catch (error) {
    throw createConfigError(error)
  }

  return {
    desktop: {
      language: config.desktop.language,
      launchAtLogin: config.desktop.launch_at_login,
      theme: config.desktop.theme,
    },
    agent: {
      codex: {
        defaultModel: config.agent.codex.default_model,
        reasoningEffort: config.agent.codex.reasoning_effort,
      },
    },
  }
}

function encodeConfig(config: LexoraConfig) {
  return {
    desktop: {
      language: config.desktop.language,
      theme: config.desktop.theme,
      launch_at_login: config.desktop.launchAtLogin,
    },
    agent: {
      codex: {
        default_model: config.agent.codex.defaultModel,
        reasoning_effort: config.agent.codex.reasoningEffort,
      },
    },
  }
}

function mergeConfig(current: LexoraConfig, patch: LexoraConfigPatch): LexoraConfig {
  return {
    desktop: {
      ...current.desktop,
      ...patch.desktop,
    },
    agent: {
      codex: {
        ...current.agent.codex,
        ...patch.agent?.codex,
      },
    },
  }
}

function mergeConfigFile(file: unknown, config: LexoraConfig): Record<string, unknown> {
  const root = asRecord(file)
  const desktop = asRecord(root.desktop)
  const agent = asRecord(root.agent)
  const codex = asRecord(agent.codex)
  const encoded = encodeConfig(config)

  return {
    ...root,
    desktop: {
      ...desktop,
      ...encoded.desktop,
    },
    agent: {
      ...agent,
      ...encoded.agent,
      codex: {
        ...codex,
        ...encoded.agent.codex,
      },
    },
  }
}

function asRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
}

function createConfigError(error: unknown): LexoraConfigError {
  const message = isZodError(error)
    ? error.issues.map(issue => `${issue.path.join('.') || 'config'}: ${issue.message}`).join('; ')
    : error instanceof Error ? error.message : 'Invalid Lexora configuration'

  return new LexoraConfigError(message, { cause: error })
}

function isZodError(error: unknown): error is ZodError {
  return error instanceof z.ZodError
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error
}
