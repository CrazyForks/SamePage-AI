import type {
  LocalAttachment,
  LocalConversation,
  LocalProject,
  LocalWorkspaceStateValue,
} from '../../electron/shared/localChatApi'
import { computed, onBeforeUnmount, readonly, shallowRef } from 'vue'
import { useBuddyI18n } from '@/i18n/buddyI18n'
import { localWorkspaceStateValueSchema } from '../../electron/shared/localChatApiSchemas'
import { normalizeDesktopError } from './desktopChatError'
import {
  hasLoadedDesktopHydrationResource,
  loadDesktopHydrationResources,
} from './desktopChatHydration'
import {
  commitAcceptedDesktopMutation,
  createCoalescingAsyncWriter,
  isDesktopChatSendAvailable,
} from './desktopChatState'
import { useDesktopApprovals } from './useDesktopApprovals'
import { useDesktopDraft } from './useDesktopDraft'
import { useDesktopRunSync } from './useDesktopRunSync'
import { useDesktopRuntime } from './useDesktopRuntime'

type ChatScope = 'global' | 'project'

export function useDesktopChat() {
  const api = requireDesktopApi()
  const writeWorkspaceState = createCoalescingAsyncWriter<LocalWorkspaceStateValue>(
    value => api.localChat.workspaceState.write(value),
  )
  const projects = shallowRef<ReadonlyArray<LocalProject>>([])
  const conversations = shallowRef<ReadonlyArray<LocalConversation>>([])
  const activeConversationId = shallowRef<string | null>(null)
  const draftScope = shallowRef<ChatScope>('global')
  const projectRoot = shallowRef<string | null>(null)
  const sidebarCollapsed = shallowRef(false)
  const isLoading = shallowRef(true)
  const isSending = shallowRef(false)
  const isSelectingFiles = shallowRef(false)
  const errorMessage = shallowRef<string | null>(null)
  let navigationGeneration = 0
  let hasInitialized = false
  let hasHydratedDrafts = false
  let hydrationPromise: Promise<boolean> | null = null

  const desktopRuntime = useDesktopRuntime({
    api,
    onReady: () => void rehydrateAfterRuntimeReady(),
  })
  const {
    agentError,
    codexStatus,
    config,
    isLoadingAgent,
    isLoadingLocalState,
    isLoadingUsage,
    language,
    loadAgent,
    loadLocalState,
    loadUsage,
    localState,
    localStateError,
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
  } = desktopRuntime
  const { t } = useBuddyI18n(language)

  const runSync = useDesktopRunSync({
    activeConversationId,
    api: api.localChat,
    onError: error => errorMessage.value = normalizeDesktopError(error, language.value),
    refreshCollections,
  })
  const { approvals, messages, runEvents, runs } = runSync

  const activeConversation = computed(() =>
    conversations.value.find(item => item.id === activeConversationId.value) ?? null,
  )
  const activeProject = computed(() =>
    projects.value.find(item => item.root === projectRoot.value) ?? null,
  )
  const activeRun = computed(() =>
    runs.value.find(run => run.status === 'running' || run.status === 'queued') ?? null,
  )
  const currentScope = computed<ChatScope>(() =>
    activeConversation.value?.scope ?? draftScope.value,
  )
  const currentCwd = computed(() =>
    activeConversation.value?.projectRoot ?? (currentScope.value === 'project' ? projectRoot.value : null),
  )
  const currentTitle = computed(() =>
    activeConversation.value?.title?.trim()
    || activeProject.value?.name
    || (currentScope.value === 'project' ? t('chat.projectsSection') : t('chat.newConversation')),
  )
  const canSend = computed(() =>
    isDesktopChatSendAvailable(
      runtimeState.value.status,
      codexStatus.value?.activeProtocol ?? null,
      activeRun.value !== null,
      isSending.value,
    ),
  )
  const draftTargetKey = computed(() => {
    if (activeConversationId.value)
      return `conversation:${activeConversationId.value}`

    return draftScope.value === 'project' && projectRoot.value
      ? `project:${projectRoot.value}`
      : 'global'
  })
  const desktopDraft = useDesktopDraft({
    cleanupDraftAttachments: attachmentIds => api.localChat.attachments.cleanupDrafts(attachmentIds),
    onChange: () => {
      if (hasHydratedDrafts)
        void persistWorkspaceState()
    },
    releaseAttachments: attachmentIds => api.localChat.attachments.release(attachmentIds),
    targetKey: draftTargetKey,
  })
  const { attachments, composerContent, draft, restoreCurrentDraft, saveCurrentDraft } = desktopDraft
  const { approvalViews, resolveApproval, resolvingApprovalIds } = useDesktopApprovals({
    api: api.localChat,
    approvals,
    onError: error => errorMessage.value = normalizeDesktopError(error, language.value),
    refresh: runSync.refreshActiveConversation,
  })

  const stopRunEvents = api.localChat.chat.onRunEvent(runSync.handleRunStateEvent)

  async function initialize() {
    const initialized = await hydrateDesktopState()
    hasInitialized = true
    desktopRuntime.markInitialized()
    if (!initialized && runtimeState.value.status === 'ready')
      void rehydrateAfterRuntimeReady()
  }

  async function rehydrateAfterRuntimeReady() {
    await hydrateDesktopState()
    void refreshActivatedResources()
  }

  function hydrateDesktopState(): Promise<boolean> {
    if (hydrationPromise)
      return hydrationPromise

    hydrationPromise = loadDesktopState().finally(() => {
      hydrationPromise = null
    })
    return hydrationPromise
  }

  async function loadDesktopState(): Promise<boolean> {
    isLoading.value = true
    errorMessage.value = null
    const runtimeSnapshotVersion = desktopRuntime.beginHydration()
    try {
      const { errors, values } = await loadDesktopHydrationResources({
        codexStatus: () => api.localChat.codex.getStatus(),
        config: () => api.settings.get(),
        conversations: () => api.localChat.conversations.list(),
        models: () => api.localChat.codex.listModels(),
        projects: () => api.localChat.projects.list(),
        runtimeState: () => api.localChat.runtime.getStatus(),
        workspaceState: () => api.localChat.workspaceState.read(),
      })

      if (values.projects)
        projects.value = values.projects
      if (values.conversations)
        conversations.value = values.conversations
      desktopRuntime.applyHydration(values, runtimeSnapshotVersion)
      if (hasInitialized)
        saveCurrentDraft()
      const hasLoadedWorkspaceState = hasLoadedDesktopHydrationResource(values, 'workspaceState')
      if (hasLoadedWorkspaceState) {
        const stored = localWorkspaceStateValueSchema.safeParse(values.workspaceState?.value ?? null)
        if (stored.success) {
          if (!hasHydratedDrafts) {
            desktopDraft.hydrate(stored.data.drafts)
            hasHydratedDrafts = true
          }
          activeConversationId.value = conversations.value.some(item => item.id === stored.data.activeConversationId)
            ? stored.data.activeConversationId
            : null
          projectRoot.value = projects.value.some(item => item.root === stored.data.projectRoot)
            ? stored.data.projectRoot
            : null
          sidebarCollapsed.value = stored.data.sidebarCollapsed
        }
        else if (!hasHydratedDrafts) {
          desktopDraft.hydrate([])
          hasHydratedDrafts = true
        }
      }

      navigationGeneration += 1
      restoreCurrentDraft()
      if (runtimeState.value.status === 'ready' && hasHydratedDrafts)
        await desktopDraft.cleanupAbandonedAttachments()
      if (activeConversationId.value)
        await runSync.refreshActiveConversation()
      else
        runSync.clearConversationState()
      errorMessage.value = errors.length
        ? normalizeDesktopError(errors[0], language.value)
        : null
      return errors.length === 0
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
      return false
    }
    finally {
      isLoading.value = false
    }
  }

  async function refreshCollections() {
    const [nextProjects, nextConversations] = await Promise.all([
      api.localChat.projects.list(),
      api.localChat.conversations.list(),
    ])
    projects.value = nextProjects
    conversations.value = nextConversations
  }

  async function openConversation(conversationId: string) {
    const conversation = conversations.value.find(item => item.id === conversationId)
    if (!conversation)
      return

    saveCurrentDraft()
    navigationGeneration += 1
    activeConversationId.value = conversation.id
    draftScope.value = conversation.scope
    projectRoot.value = conversation.projectRoot
    errorMessage.value = null
    runSync.clearConversationState()
    restoreCurrentDraft()
    await persistWorkspaceState()
    await runSync.refreshActiveConversation()
  }

  async function startGlobalConversation() {
    switchDraftTarget('global', null)
    await persistWorkspaceState()
  }

  async function startProjectConversation(root: string) {
    if (!projects.value.some(project => project.root === root))
      return

    switchDraftTarget('project', root)
    await persistWorkspaceState()
  }

  async function authorizeProject() {
    try {
      const project = await api.localChat.projects.authorize()
      if (!project)
        return

      await refreshCollections()
      await startProjectConversation(project.root)
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
    }
  }

  async function deleteConversation(conversationId: string) {
    try {
      await api.localChat.conversations.delete(conversationId)
      await desktopDraft.discard(`conversation:${conversationId}`)
      if (activeConversationId.value === conversationId)
        switchDraftTarget('global', null, false)
      await refreshCollections()
      await persistWorkspaceState()
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
    }
  }

  async function selectAttachments() {
    if (isLoading.value)
      return

    const remainingCount = 16 - attachments.value.length
    if (remainingCount <= 0) {
      errorMessage.value = t('desktop.chat.attachmentLimit')
      return
    }

    isSelectingFiles.value = true
    try {
      const rejectedCount = await desktopDraft.appendAttachments(
        await api.localChat.attachments.selectFiles({ remainingCount }),
      )
      if (rejectedCount > 0)
        errorMessage.value = t('desktop.chat.attachmentLimit')
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
    }
    finally {
      isSelectingFiles.value = false
    }
  }

  function listContextOptions(fileQuery: string | null) {
    return api.localChat.codex.listContextOptions({
      cwd: currentCwd.value,
      fileQuery,
    })
  }

  function listRecentRuns() {
    return api.localChat.runs.list({ limit: 60 })
  }

  function listRunEvents(runId: string) {
    return api.localChat.runs.listChatEvents({ runId, limit: 300 })
  }

  async function removeAttachment(index: number) {
    try {
      await desktopDraft.removeAttachment(index)
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
    }
  }

  function updateDraft(content: string) {
    desktopDraft.updateDraft(content)
  }

  function updateComposerContent(content: string, value: Parameters<typeof desktopDraft.updateComposerContent>[1]) {
    desktopDraft.updateComposerContent(content, value)
  }

  async function send(payload: string | {
    content: string
    contextItems: ReadonlyArray<NonNullable<Parameters<typeof api.localChat.chat.startTurn>[0]['contextItems']>[number]>
    inputs: ReadonlyArray<NonNullable<Parameters<typeof api.localChat.chat.startTurn>[0]['inputs']>[number]>
  }) {
    const content = typeof payload === 'string' ? payload : payload.content
    const contextItems = typeof payload === 'string' ? [] : payload.contextItems
    const inputs = typeof payload === 'string' ? [] : payload.inputs
    const text = content.trim()
    if ((!text && attachments.value.length === 0) || !canSend.value)
      return false

    const sourceKey = draftTargetKey.value
    saveCurrentDraft()
    const model = selectedModel.value
    const modelSelection = model
      ? {
          runtime: 'codex' as const,
          model: model.model,
          serviceTier: selectedServiceTier.value,
          effort: selectedEffort.value,
        }
      : null
    const sourceDraft = desktopDraft.prepareSend(
      crypto.randomUUID(),
      JSON.stringify(modelSelection),
    )
    const sourceNavigationGeneration = navigationGeneration
    const sourceConversationId = activeConversationId.value
    const sourceScope = currentScope.value
    const sourceCwd = currentCwd.value
    isSending.value = true
    errorMessage.value = null
    try {
      const result = await api.localChat.chat.startTurn({
        requestId: sourceDraft.requestId!,
        attachments: sourceDraft.attachments.map(attachment => ({
          attachmentId: attachment.attachmentId,
        })),
        content: text,
        contextItems: [...contextItems],
        conversationId: sourceConversationId,
        conversationSeed: sourceConversationId
          ? null
          : {
              scope: sourceScope,
              projectRoot: sourceCwd,
              title: createConversationTitle(text),
              sourceConversationId: null,
              forkedFromMessageId: null,
              sourceRunId: null,
            },
        cwd: sourceCwd,
        inputs: [...inputs],
        modelSelection,
      })

      const storedDraft = desktopDraft.load(sourceKey)
      await commitAcceptedDesktopMutation({
        commit() {
          conversations.value = upsertConversation(conversations.value, result.conversation)
          if (!sourceConversationId) {
            desktopDraft.reconcileAfterSend(
              sourceKey,
              `conversation:${result.conversation.id}`,
              sourceDraft,
            )
          }
          else if (draftsMatch(sourceDraft, storedDraft)) {
            desktopDraft.clear(sourceKey)
          }

          if (
            sourceNavigationGeneration !== navigationGeneration
            || sourceKey !== draftTargetKey.value
          ) {
            return
          }

          activeConversationId.value = result.conversation.id
          draftScope.value = result.conversation.scope
          projectRoot.value = result.conversation.projectRoot
          runSync.applyTurnStart(result)
          restoreCurrentDraft()
        },
        onReconcileError: (error) => {
          errorMessage.value = normalizeDesktopError(error, language.value)
        },
        reconcile: [
          refreshCollections,
          persistWorkspaceState,
          async () => {
            if (activeConversationId.value === result.conversation.id)
              await runSync.refreshActiveConversation()
          },
        ],
      })
      return true
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
      return false
    }
    finally {
      isSending.value = false
    }
  }

  async function cancelActiveRun() {
    const run = activeRun.value
    if (!run)
      return

    try {
      const nextRun = await api.localChat.chat.cancel(run.id)
      runSync.upsertRuns([nextRun])
      await runSync.refreshActiveConversation()
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
    }
  }

  async function restartChatRuntime() {
    if (await restartRuntime())
      return

    errorMessage.value = agentError.value
  }

  async function setSidebarCollapsed(value: boolean) {
    sidebarCollapsed.value = value
    await persistWorkspaceState()
  }

  function switchDraftTarget(scope: ChatScope, root: string | null, preserveCurrent = true) {
    if (preserveCurrent)
      saveCurrentDraft()
    navigationGeneration += 1
    activeConversationId.value = null
    draftScope.value = scope
    projectRoot.value = root
    runSync.clearConversationState()
    restoreCurrentDraft()
    errorMessage.value = null
  }

  async function persistWorkspaceState() {
    try {
      await writeWorkspaceState({
        activeConversationId: activeConversationId.value,
        drafts: desktopDraft.exportDrafts(),
        projectRoot: projectRoot.value,
        sidebarCollapsed: sidebarCollapsed.value,
      })
    }
    catch (error) {
      errorMessage.value = normalizeDesktopError(error, language.value)
    }
  }

  onBeforeUnmount(() => {
    desktopRuntime.dispose()
    stopRunEvents()
    runSync.dispose()
  })

  return {
    activeConversation: readonly(activeConversation),
    activeConversationId: readonly(activeConversationId),
    activeRun: readonly(activeRun),
    approvalViews: readonly(approvalViews),
    approvals: readonly(approvals),
    attachments: readonly(attachments),
    authorizeProject,
    agentError: readonly(agentError),
    canSend: readonly(canSend),
    cancelActiveRun,
    codexStatus: readonly(codexStatus),
    composerContent: readonly(composerContent),
    config: readonly(config),
    conversations: readonly(conversations),
    currentCwd: readonly(currentCwd),
    currentScope: readonly(currentScope),
    currentTitle: readonly(currentTitle),
    deleteConversation,
    draft: readonly(draft),
    errorMessage: readonly(errorMessage),
    initialize,
    isLoading: readonly(isLoading),
    isLoadingAgent: readonly(isLoadingAgent),
    isLoadingLocalState: readonly(isLoadingLocalState),
    isLoadingUsage: readonly(isLoadingUsage),
    isSelectingFiles: readonly(isSelectingFiles),
    isSending: readonly(isSending),
    language: readonly(language),
    loadAgent,
    loadLocalState,
    loadUsage,
    listContextOptions,
    listRecentRuns,
    listRunEvents,
    localState: readonly(localState),
    localStateError: readonly(localStateError),
    getAppInfo: () => api.app.getInfo(),
    messages: readonly(messages),
    models: readonly(models),
    openConversation,
    projects: readonly(projects),
    projectRoot: readonly(projectRoot),
    removeAttachment,
    resolveApproval,
    resolvingApprovalIds: readonly(resolvingApprovalIds),
    restartRuntime,
    restartChatRuntime,
    runEvents: readonly(runEvents),
    runtimeState: readonly(runtimeState),
    selectAttachments,
    selectedEffort,
    selectedModel: readonly(selectedModel),
    selectedModelId: readonly(selectedModelId),
    selectedServiceTier,
    selectModel,
    send,
    setSidebarCollapsed,
    sidebarCollapsed: readonly(sidebarCollapsed),
    startGlobalConversation,
    startProjectConversation,
    settingsError: readonly(settingsError),
    updateDraft,
    updateComposerContent,
    updateSettings,
    usageError: readonly(usageError),
    usageSnapshot: readonly(usageSnapshot),
  }
}

export type DesktopChatController = ReturnType<typeof useDesktopChat>

function upsertConversation(
  conversations: ReadonlyArray<LocalConversation>,
  incoming: LocalConversation,
): ReadonlyArray<LocalConversation> {
  return [incoming, ...conversations.filter(conversation => conversation.id !== incoming.id)]
}

function requireDesktopApi() {
  if (!window.lexoraDesktop)
    throw new Error('Lexora Desktop bridge is unavailable')

  return window.lexoraDesktop
}

function createConversationTitle(content: string): string | null {
  const title = content.replace(/\s+/g, ' ').trim().slice(0, 48)
  return title || null
}

function draftsMatch(
  left: { attachments: ReadonlyArray<LocalAttachment>, content: string },
  right: { attachments: ReadonlyArray<LocalAttachment>, content: string },
): boolean {
  return left.content === right.content
    && left.attachments.length === right.attachments.length
    && left.attachments.every((attachment, index) =>
      attachment.attachmentId === right.attachments[index]?.attachmentId,
    )
}
