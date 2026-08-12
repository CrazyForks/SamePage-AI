import type {
  LexoraConfig,
  LexoraConfigPatch,
  LexoraDesktopApi,
} from '../../electron/shared/desktopApi'
import type {
  LocalCodexRuntimeStatus,
  LocalRuntimeModelOption,
  LocalRuntimeSupervisorState,
  LocalStateStatus,
  LocalUsageSnapshot,
} from '../../electron/shared/localChatApi'
import type { DesktopHydrationValues } from './desktopChatHydration'
import type { BuddyLocale } from '@/i18n/buddyI18n'
import { computed, shallowRef } from 'vue'
import { resolveBuddyLocale, translateBuddy } from '@/i18n/buddyI18n'
import { normalizeDesktopError } from './desktopChatError'
import {
  isDesktopRuntimeSnapshotCurrent,
  isRuntimeReadyTransition,
} from './desktopChatState'

interface UseDesktopRuntimeOptions {
  api: LexoraDesktopApi
  onReady: () => void
}

export function useDesktopRuntime(options: UseDesktopRuntimeOptions) {
  const runtimeState = shallowRef<LocalRuntimeSupervisorState>({
    status: 'stopped',
    pid: null,
    restartAttempt: 0,
    lastError: null,
  })
  const codexStatus = shallowRef<LocalCodexRuntimeStatus | null>(null)
  const localState = shallowRef<LocalStateStatus | null>(null)
  const usageSnapshot = shallowRef<LocalUsageSnapshot | null>(null)
  const models = shallowRef<ReadonlyArray<LocalRuntimeModelOption>>([])
  const config = shallowRef<LexoraConfig | null>(null)
  const language = shallowRef<BuddyLocale>('zh-CN')
  const selectedModelId = shallowRef<string | null>(null)
  const selectedEffort = shallowRef<string | null>(null)
  const selectedServiceTier = shallowRef<string | null>(null)
  const isLoadingUsage = shallowRef(false)
  const isLoadingAgent = shallowRef(false)
  const isLoadingLocalState = shallowRef(false)
  const usageError = shallowRef<string | null>(null)
  const agentError = shallowRef<string | null>(null)
  const localStateError = shallowRef<string | null>(null)
  const settingsError = shallowRef<string | null>(null)
  let initialized = false
  let runtimeStateVersion = 0
  let usagePromise: Promise<boolean> | null = null
  let agentPromise: Promise<boolean> | null = null
  let localStatePromise: Promise<boolean> | null = null
  let hasUsageSnapshot = false
  let hasAgentSnapshot = false
  let hasLocalStateSnapshot = false
  let hasActivatedUsage = false
  let hasActivatedAgent = false
  let hasActivatedLocalState = false

  const selectedModel = computed(() =>
    models.value.find(model => model.id === selectedModelId.value) ?? null,
  )
  const stopRuntimeState = options.api.localChat.runtime.onStateChanged((state) => {
    const previousStatus = runtimeState.value.status
    runtimeStateVersion += 1
    runtimeState.value = state
    if (isRuntimeReadyTransition(previousStatus, state.status, initialized))
      options.onReady()
  })

  function applyHydration(values: DesktopHydrationValues, runtimeSnapshotVersion: number) {
    if (values.models)
      models.value = values.models
    if (values.config)
      applyConfig(values.config)
    if (values.codexStatus) {
      codexStatus.value = values.codexStatus
      hasAgentSnapshot = true
    }
    if (values.runtimeState && isDesktopRuntimeSnapshotCurrent(runtimeSnapshotVersion, runtimeStateVersion))
      runtimeState.value = values.runtimeState
  }

  function beginHydration() {
    codexStatus.value = null
    hasAgentSnapshot = false
    return runtimeStateVersion
  }

  function markInitialized() {
    initialized = true
  }

  async function restartRuntime() {
    const snapshotVersion = runtimeStateVersion
    agentError.value = null
    try {
      const state = await options.api.localChat.runtime.restart()
      if (isDesktopRuntimeSnapshotCurrent(snapshotVersion, runtimeStateVersion))
        runtimeState.value = state
      return true
    }
    catch (error) {
      agentError.value = normalizeDesktopError(error, language.value)
      return false
    }
  }

  async function updateSettings(patch: LexoraConfigPatch) {
    settingsError.value = null
    try {
      applyConfig(await options.api.settings.update(patch))
      return true
    }
    catch {
      settingsError.value = translateBuddy(language.value, 'desktop.settings.saveFailed')
      return false
    }
  }

  function loadUsage(force = false) {
    hasActivatedUsage = true
    if (!force && hasUsageSnapshot)
      return Promise.resolve(true)
    if (usagePromise)
      return usagePromise

    usagePromise = collectUsage().finally(() => {
      usagePromise = null
    })
    return usagePromise
  }

  async function collectUsage() {
    isLoadingUsage.value = true
    usageError.value = null
    try {
      usageSnapshot.value = await options.api.localChat.usage.getSnapshot()
      hasUsageSnapshot = true
      return true
    }
    catch (error) {
      usageError.value = normalizeDesktopError(error, language.value)
      return false
    }
    finally {
      isLoadingUsage.value = false
    }
  }

  function loadAgent(force = false) {
    hasActivatedAgent = true
    if (!force && hasAgentSnapshot)
      return Promise.resolve(true)
    if (agentPromise)
      return agentPromise

    agentPromise = collectAgent().finally(() => {
      agentPromise = null
    })
    return agentPromise
  }

  async function collectAgent() {
    isLoadingAgent.value = true
    agentError.value = null
    try {
      codexStatus.value = await options.api.localChat.codex.getStatus()
      hasAgentSnapshot = true
      return true
    }
    catch (error) {
      agentError.value = normalizeDesktopError(error, language.value)
      return false
    }
    finally {
      isLoadingAgent.value = false
    }
  }

  function loadLocalState(force = false) {
    hasActivatedLocalState = true
    if (!force && hasLocalStateSnapshot)
      return Promise.resolve(true)
    if (localStatePromise)
      return localStatePromise

    localStatePromise = collectLocalState().finally(() => {
      localStatePromise = null
    })
    return localStatePromise
  }

  async function collectLocalState() {
    isLoadingLocalState.value = true
    localStateError.value = null
    try {
      localState.value = await options.api.localChat.runtime.getLocalState()
      hasLocalStateSnapshot = true
      return true
    }
    catch (error) {
      localStateError.value = normalizeDesktopError(error, language.value)
      return false
    }
    finally {
      isLoadingLocalState.value = false
    }
  }

  async function refreshActivatedResources() {
    const loaders: Array<Promise<boolean>> = []
    if (hasActivatedUsage)
      loaders.push(loadUsage(true))
    if (hasActivatedAgent)
      loaders.push(loadAgent(true))
    if (hasActivatedLocalState)
      loaders.push(loadLocalState(true))
    await Promise.all(loaders)
  }

  function selectModel(modelId: string) {
    const model = models.value.find(item => item.id === modelId)
    if (!model)
      return

    selectedModelId.value = model.id
    selectedEffort.value = model.defaultReasoningEffort
    selectedServiceTier.value = model.defaultServiceTier
  }

  function applyConfig(nextConfig: LexoraConfig) {
    config.value = nextConfig
    language.value = resolveBuddyLocale(nextConfig.desktop.language)
    document.documentElement.lang = language.value
    const configuredModel = models.value.find(model => model.model === nextConfig.agent.codex.defaultModel)
    const fallbackModel = configuredModel ?? models.value.find(model => model.isDefault) ?? models.value[0] ?? null
    selectedModelId.value = fallbackModel?.id ?? null
    selectedEffort.value = normalizeSelection(
      nextConfig.agent.codex.reasoningEffort,
      fallbackModel?.supportedReasoningEfforts.map(option => option.reasoningEffort) ?? [],
    ) ?? fallbackModel?.defaultReasoningEffort ?? null
    selectedServiceTier.value = fallbackModel?.defaultServiceTier ?? null
  }

  return {
    agentError,
    applyHydration,
    beginHydration,
    codexStatus,
    config,
    dispose: stopRuntimeState,
    isLoadingAgent,
    isLoadingLocalState,
    isLoadingUsage,
    language,
    loadAgent,
    loadLocalState,
    loadUsage,
    localState,
    localStateError,
    markInitialized,
    models,
    refreshActivatedResources,
    restartRuntime,
    runtimeState,
    selectedEffort,
    selectedModel,
    selectedModelId,
    selectedServiceTier,
    selectModel,
    settingsError,
    updateSettings,
    usageError,
    usageSnapshot,
  }
}

function normalizeSelection(value: string, options: ReadonlyArray<string>): string | null {
  return options.includes(value) ? value : null
}
