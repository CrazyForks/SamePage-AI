import type { Ref } from 'vue'
import type { LocalApproval, LocalChatApi, LocalMessage, LocalRun, LocalRunEvent, LocalRunStateEvent, LocalTurnStart } from '../../electron/shared/localChatApi'
import { shallowRef } from 'vue'
import { getRunSyncRetryDelay, mergeLocalRunEvents } from './desktopChatState'

const RUN_EVENT_LIMIT = 1_000
const INITIAL_RUN_EVENT_LIMIT = 500
const RUN_EVENT_BATCH_LIMIT = 200
const RUN_SYNC_INTERVAL_MS = 80

interface DesktopRunSyncOptions {
  activeConversationId: Ref<string | null>
  api: LocalChatApi
  onError: (error: unknown) => void
  refreshCollections: () => Promise<void>
}

export function useDesktopRunSync(options: DesktopRunSyncOptions) {
  const messages = shallowRef<ReadonlyArray<LocalMessage>>([])
  const runs = shallowRef<ReadonlyArray<LocalRun>>([])
  const runEvents = shallowRef<ReadonlyArray<LocalRunEvent>>([])
  const approvals = shallowRef<ReadonlyArray<LocalApproval>>([])
  const runEventCursors = new Map<string, number>()
  const pendingRunIds = new Set<string>()
  const runSyncRetryAttempts = new Map<string, number>()
  let refreshGeneration = 0
  let runSyncTimer: number | null = null
  let isRunSyncing = false
  let conversationRefreshCount = 0

  async function refreshActiveConversation() {
    const conversationId = options.activeConversationId.value
    const generation = ++refreshGeneration
    if (!conversationId) {
      clearConversationState()
      return
    }

    conversationRefreshCount += 1
    try {
      const [nextMessages, nextRuns, nextEvents, pendingApprovals] = await Promise.all([
        options.api.conversations.listMessages({ conversationId, limit: 100 }),
        options.api.runs.list({ conversationId, limit: 100 }),
        options.api.runs.listConversationChatEvents({
          conversationId,
          eventLimit: INITIAL_RUN_EVENT_LIMIT,
          runLimit: 40,
        }),
        options.api.approvals.list({ status: 'pending', limit: 100 }),
      ])
      if (generation !== refreshGeneration || conversationId !== options.activeConversationId.value)
        return

      const runIds = new Set(nextRuns.map(run => run.id))
      messages.value = nextMessages
      runs.value = nextRuns
      runEvents.value = nextEvents.slice(-RUN_EVENT_LIMIT)
      approvals.value = pendingApprovals.filter(approval => approval.runId && runIds.has(approval.runId))
      replaceRunEventCursors(nextEvents)
    }
    catch (error) {
      options.onError(error)
    }
    finally {
      conversationRefreshCount -= 1
      if (conversationRefreshCount === 0 && pendingRunIds.size)
        scheduleRunSync()
    }
  }

  function handleRunStateEvent(event: LocalRunStateEvent) {
    pendingRunIds.add(event.runId)
    scheduleRunSync()
  }

  function scheduleRunSync(delayMs = RUN_SYNC_INTERVAL_MS) {
    if (runSyncTimer !== null || isRunSyncing || conversationRefreshCount > 0)
      return

    runSyncTimer = window.setTimeout(() => {
      runSyncTimer = null
      void flushRunUpdates()
    }, delayMs)
  }

  async function flushRunUpdates() {
    const conversationId = options.activeConversationId.value
    if (!conversationId || pendingRunIds.size === 0)
      return

    const runIds = [...pendingRunIds]
    pendingRunIds.clear()
    isRunSyncing = true
    try {
      const settledUpdates = await Promise.allSettled(runIds.map(async (runId) => {
        const [run, events] = await Promise.all([
          options.api.runs.get(runId),
          options.api.runs.listChatEvents({
            runId,
            afterId: runEventCursors.get(runId) ?? null,
            limit: RUN_EVENT_BATCH_LIMIT,
          }),
        ])
        return { events, run }
      }))

      if (conversationId !== options.activeConversationId.value)
        return

      const updates: Array<{ events: ReadonlyArray<LocalRunEvent>, run: LocalRun }> = []
      for (const [index, result] of settledUpdates.entries()) {
        const runId = runIds[index]!
        if (result.status === 'fulfilled') {
          runSyncRetryAttempts.delete(runId)
          updates.push(result.value)
          continue
        }

        const retryAttempt = (runSyncRetryAttempts.get(runId) ?? 0) + 1
        if (retryAttempt <= 4) {
          runSyncRetryAttempts.set(runId, retryAttempt)
          pendingRunIds.add(runId)
        }
        else {
          runSyncRetryAttempts.delete(runId)
        }
        options.onError(result.reason)
      }

      const matchingUpdates = updates.filter(update => update.run.conversationId === conversationId)
      const nextRuns = matchingUpdates.map(update => update.run)
      const nextEvents = matchingUpdates.flatMap(update => update.events)
      if (nextRuns.length)
        upsertRuns(nextRuns)
      if (nextEvents.length) {
        runEvents.value = mergeLocalRunEvents(runEvents.value, nextEvents, RUN_EVENT_LIMIT)
        updateRunEventCursors(nextEvents)
      }
      for (const update of matchingUpdates) {
        if (update.events.length === RUN_EVENT_BATCH_LIMIT)
          pendingRunIds.add(update.run.id)
      }

      await refreshPendingApprovals(conversationId)
      if (nextRuns.some(run => isTerminalRun(run.status))) {
        await Promise.all([
          refreshMessages(conversationId),
          options.refreshCollections(),
        ])
      }
    }
    catch (error) {
      options.onError(error)
    }
    finally {
      isRunSyncing = false
      if (pendingRunIds.size) {
        const retryAttempt = Math.min(
          ...[...pendingRunIds].map(runId => runSyncRetryAttempts.get(runId) ?? 0),
        )
        scheduleRunSync(retryAttempt > 0
          ? getRunSyncRetryDelay(retryAttempt)
          : RUN_SYNC_INTERVAL_MS)
      }
    }
  }

  function upsertRuns(incoming: ReadonlyArray<LocalRun>) {
    runs.value = mergeRuns(runs.value, incoming)
  }

  function applyTurnStart(turn: LocalTurnStart) {
    if (turn.conversation.id !== options.activeConversationId.value)
      return

    messages.value = mergeMessages(
      messages.value,
      [turn.userMessage, ...(turn.assistantMessage ? [turn.assistantMessage] : [])],
    )
    if (turn.run)
      upsertRuns([turn.run])
  }

  function clearConversationState() {
    refreshGeneration += 1
    messages.value = []
    runs.value = []
    runEvents.value = []
    approvals.value = []
    runEventCursors.clear()
    pendingRunIds.clear()
    runSyncRetryAttempts.clear()
    if (runSyncTimer !== null) {
      window.clearTimeout(runSyncTimer)
      runSyncTimer = null
    }
  }

  function replaceRunEventCursors(events: ReadonlyArray<LocalRunEvent>) {
    runEventCursors.clear()
    updateRunEventCursors(events)
  }

  function updateRunEventCursors(events: ReadonlyArray<LocalRunEvent>) {
    for (const event of events)
      runEventCursors.set(event.runId, Math.max(runEventCursors.get(event.runId) ?? 0, event.id))
  }

  async function refreshMessages(conversationId: string) {
    const nextMessages = await options.api.conversations.listMessages({ conversationId, limit: 100 })
    if (conversationId === options.activeConversationId.value)
      messages.value = nextMessages
  }

  async function refreshPendingApprovals(conversationId: string) {
    const pendingApprovals = await options.api.approvals.list({ status: 'pending', limit: 100 })
    if (conversationId !== options.activeConversationId.value)
      return

    const runIds = new Set(runs.value.map(run => run.id))
    approvals.value = pendingApprovals.filter(approval => approval.runId && runIds.has(approval.runId))
  }

  function dispose() {
    if (runSyncTimer !== null)
      window.clearTimeout(runSyncTimer)
  }

  return {
    approvals,
    applyTurnStart,
    clearConversationState,
    dispose,
    handleRunStateEvent,
    messages,
    refreshActiveConversation,
    runEvents,
    runs,
    upsertRuns,
  }
}

function mergeMessages(
  current: ReadonlyArray<LocalMessage>,
  incoming: ReadonlyArray<LocalMessage>,
): ReadonlyArray<LocalMessage> {
  const byId = new Map(current.map(message => [message.id, message]))
  for (const message of incoming)
    byId.set(message.id, message)

  return [...byId.values()]
    .sort((left, right) => left.createdAt.localeCompare(right.createdAt))
    .slice(-100)
}

function mergeRuns(
  current: ReadonlyArray<LocalRun>,
  incoming: ReadonlyArray<LocalRun>,
): ReadonlyArray<LocalRun> {
  const byId = new Map(current.map(run => [run.id, run]))
  for (const run of incoming)
    byId.set(run.id, run)

  return [...byId.values()]
    .sort((left, right) => right.startedAt.localeCompare(left.startedAt))
    .slice(0, 100)
}

function isTerminalRun(status: LocalRun['status']): boolean {
  return status === 'completed' || status === 'failed' || status === 'cancelled'
}
