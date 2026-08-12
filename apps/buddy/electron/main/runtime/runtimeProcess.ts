import type { RuntimeChildProcess } from './RuntimeSupervisor'
import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { isAbsolute, join } from 'node:path'

export interface RuntimeExecutableOptions {
  appPath: string
  isPackaged: boolean
  resourcesPath: string
  runtimePathOverride?: string
}

export interface RuntimeSpawnOptions {
  env: NodeJS.ProcessEnv
  stdio: 'pipe'
  windowsHide: true
}

export interface RuntimeProcessFactoryOptions extends RuntimeExecutableOptions {
  env: NodeJS.ProcessEnv
  exists?: (path: string) => boolean
  spawnRuntime?: (
    command: string,
    args: string[],
    options: RuntimeSpawnOptions,
  ) => RuntimeChildProcess
}

export function resolveRuntimeExecutable(options: RuntimeExecutableOptions): string {
  if (options.runtimePathOverride) {
    if (!isAbsolute(options.runtimePathOverride))
      throw new Error('LEXORA_BUDDY_RUNTIME_PATH must be an absolute path')

    return options.runtimePathOverride
  }

  const executable = 'lexora-buddy-runtime'

  if (options.isPackaged)
    return join(options.resourcesPath, 'runtime', executable)

  return join(options.appPath, 'runtime', 'target', 'debug', executable)
}

export function createRuntimeProcessFactory(
  options: RuntimeProcessFactoryOptions,
): () => RuntimeChildProcess {
  const executable = resolveRuntimeExecutable(options)
  const exists = options.exists ?? existsSync
  const spawnRuntime = options.spawnRuntime ?? ((command, args, spawnOptions) => {
    return spawn(command, args, spawnOptions)
  })

  return () => {
    if (!exists(executable))
      throw new Error(`Lexora Runtime executable not found: ${executable}`)

    return spawnRuntime(executable, [], {
      env: options.env,
      stdio: 'pipe',
      windowsHide: true,
    })
  }
}
