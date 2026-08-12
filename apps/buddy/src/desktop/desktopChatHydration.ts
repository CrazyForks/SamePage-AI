import type { LexoraConfig } from '../../electron/shared/desktopApi'
import type {
  LocalCodexRuntimeStatus,
  LocalConversation,
  LocalProject,
  LocalRuntimeModelOption,
  LocalRuntimeSupervisorState,
  LocalWorkspaceSetting,
} from '../../electron/shared/localChatApi'

export interface DesktopHydrationLoaders {
  codexStatus: () => Promise<LocalCodexRuntimeStatus>
  config: () => Promise<LexoraConfig>
  conversations: () => Promise<ReadonlyArray<LocalConversation>>
  models: () => Promise<ReadonlyArray<LocalRuntimeModelOption>>
  projects: () => Promise<ReadonlyArray<LocalProject>>
  runtimeState: () => Promise<LocalRuntimeSupervisorState>
  workspaceState: () => Promise<LocalWorkspaceSetting | null>
}

export type DesktopHydrationValues = Partial<{
  [Key in keyof DesktopHydrationLoaders]: Awaited<ReturnType<DesktopHydrationLoaders[Key]>>
}>

export function hasLoadedDesktopHydrationResource(
  values: DesktopHydrationValues,
  key: keyof DesktopHydrationLoaders,
): boolean {
  return Object.hasOwn(values, key)
}

export async function loadDesktopHydrationResources(
  loaders: DesktopHydrationLoaders,
): Promise<{ errors: ReadonlyArray<unknown>, values: DesktopHydrationValues }> {
  const entries = Object.entries(loaders) as Array<[
    keyof DesktopHydrationLoaders,
    DesktopHydrationLoaders[keyof DesktopHydrationLoaders],
  ]>
  const settled = await Promise.allSettled(entries.map(([, load]) => load()))
  const errors: unknown[] = []
  const values: DesktopHydrationValues = {}

  for (const [index, result] of settled.entries()) {
    if (result.status === 'rejected') {
      errors.push(result.reason)
      continue
    }

    const key = entries[index]![0]
    values[key] = result.value as never
  }

  return { errors, values }
}
