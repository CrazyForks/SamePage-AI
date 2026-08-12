import type {
  LocalApproval,
  LocalAttachment,
  LocalCodexRuntimeStatus,
  LocalRunEvent,
  LocalRuntimeSupervisorState,
  LocalWorkspaceDraft,
} from '../../electron/shared/localChatApi'

export interface DesktopDraftSnapshot {
  attachments: ReadonlyArray<LocalAttachment>
  composerContent: LocalWorkspaceDraft['composerContent']
  content: string
  requestFingerprint: string | null
  requestId: string | null
}

interface DesktopDraftInput {
  attachments: ReadonlyArray<LocalAttachment>
  composerContent?: LocalWorkspaceDraft['composerContent']
  content: string
  requestFingerprint?: string | null
  requestId?: string | null
}

export interface DesktopDraftStore {
  attachmentIds: () => ReadonlyArray<string>
  clear: (key: string) => void
  exportDrafts: () => ReadonlyArray<LocalWorkspaceDraft>
  hydrate: (drafts: ReadonlyArray<LocalWorkspaceDraft>) => void
  load: (key: string) => DesktopDraftSnapshot
  prepareSend: (key: string, requestId: string, requestFingerprint: string) => DesktopDraftSnapshot
  save: (key: string, draft: DesktopDraftInput) => void
}

export interface DesktopApprovalView {
  approval: LocalApproval
  approvalMode: 'codex' | null
  authorizationRoot: string | null
  cwd: string | null
  method: string | null
  operation: 'command' | 'file_change' | 'local'
  preview: string
  scopeReason: string | null
  scopeStatus: string | null
  targetRoot: string | null
}

const EMPTY_DRAFT: DesktopDraftSnapshot = Object.freeze({
  attachments: Object.freeze([]),
  composerContent: null,
  content: '',
  requestFingerprint: null,
  requestId: null,
})
const RUN_SYNC_RETRY_DELAYS_MS = [250, 1_000, 3_000, 10_000] as const

export function createSerialAsyncWriter<T>(
  writer: (value: T) => Promise<unknown>,
): (value: T) => Promise<void> {
  let tail = Promise.resolve()

  return (value) => {
    const operation = tail.then(async () => {
      await writer(value)
    })
    tail = operation.catch(() => undefined)
    return operation
  }
}

export function createCoalescingAsyncWriter<T>(
  writer: (value: T) => Promise<unknown>,
): (value: T) => Promise<void> {
  let isWriting = false
  let hasPendingValue = false
  let pendingValue: T
  let pendingWaiters: Array<{
    reject: (error: unknown) => void
    resolve: () => void
  }> = []

  async function drain() {
    while (hasPendingValue) {
      const value = pendingValue
      const waiters = pendingWaiters
      hasPendingValue = false
      pendingWaiters = []
      try {
        await writer(value)
        for (const waiter of waiters)
          waiter.resolve()
      }
      catch (error) {
        for (const waiter of waiters)
          waiter.reject(error)
      }
    }
    isWriting = false
  }

  return value => new Promise<void>((resolve, reject) => {
    pendingValue = value
    hasPendingValue = true
    pendingWaiters.push({ reject, resolve })
    if (isWriting)
      return

    isWriting = true
    void drain()
  })
}

export function getRunSyncRetryDelay(attempt: number): number {
  const index = Math.min(Math.max(1, attempt), RUN_SYNC_RETRY_DELAYS_MS.length) - 1
  return RUN_SYNC_RETRY_DELAYS_MS[index]!
}

export function createDesktopDraftStore(): DesktopDraftStore {
  const drafts = new Map<string, DesktopDraftSnapshot>()

  return {
    attachmentIds() {
      return [...new Set(
        [...drafts.values()].flatMap(draft =>
          draft.attachments.map(attachment => attachment.attachmentId),
        ),
      )]
    },
    clear(key) {
      drafts.delete(key)
    },
    exportDrafts() {
      return [...drafts.entries()]
        .map(([targetKey, draft]) => ({
          attachments: draft.attachments.map(attachment => ({
            attachmentId: attachment.attachmentId,
            kind: attachment.kind,
            mimeType: attachment.mimeType,
            name: attachment.name,
            sizeBytes: attachment.sizeBytes,
          })),
          composerContent: draft.composerContent,
          content: draft.content,
          requestFingerprint: draft.requestFingerprint,
          requestId: draft.requestId,
          targetKey,
        }))
        .sort((left, right) => left.targetKey.localeCompare(right.targetKey))
    },
    hydrate(persistedDrafts) {
      drafts.clear()
      for (const draft of persistedDrafts) {
        drafts.set(draft.targetKey, {
          attachments: draft.attachments.map(attachment => ({
            ...attachment,
            dataUrl: null,
            previewPath: null,
            text: null,
          })),
          composerContent: draft.composerContent,
          content: draft.content,
          requestFingerprint: draft.requestFingerprint,
          requestId: draft.requestId,
        })
      }
    },
    load(key) {
      const draft = drafts.get(key) ?? EMPTY_DRAFT
      return {
        attachments: [...draft.attachments],
        composerContent: draft.composerContent,
        content: draft.content,
        requestFingerprint: draft.requestFingerprint,
        requestId: draft.requestId ?? null,
      }
    },
    prepareSend(key, requestId, requestFingerprint) {
      const draft = this.load(key)
      if (draft.requestId && draft.requestFingerprint === requestFingerprint)
        return draft

      const prepared = { ...draft, requestFingerprint, requestId }
      this.save(key, prepared)
      return prepared
    },
    save(key, draft) {
      drafts.set(key, {
        attachments: [...draft.attachments],
        composerContent: draft.composerContent ?? null,
        content: draft.content,
        requestFingerprint: draft.requestFingerprint ?? null,
        requestId: draft.requestId ?? null,
      })
    },
  }
}

export async function commitAcceptedDesktopMutation(options: {
  commit: () => void
  onReconcileError: (error: unknown) => void
  reconcile: ReadonlyArray<() => Promise<unknown>>
}): Promise<void> {
  options.commit()
  const results = await Promise.allSettled(
    options.reconcile.map(task => Promise.resolve().then(task)),
  )
  for (const result of results) {
    if (result.status === 'rejected')
      options.onReconcileError(result.reason)
  }
}

export function reconcileDesktopDraftAfterSend(
  drafts: DesktopDraftStore,
  sourceKey: string,
  conversationKey: string,
  sentDraft: DesktopDraftInput,
): void {
  const currentDraft = drafts.load(sourceKey)
  drafts.clear(sourceKey)
  if (!draftsMatch(sentDraft, currentDraft))
    drafts.save(conversationKey, currentDraft)
}

export function isRuntimeReadyTransition(
  previousStatus: LocalRuntimeSupervisorState['status'],
  nextStatus: LocalRuntimeSupervisorState['status'],
  initialized: boolean,
): boolean {
  return initialized && previousStatus !== 'ready' && nextStatus === 'ready'
}

export function isDesktopRuntimeSnapshotCurrent(
  snapshotVersion: number,
  currentVersion: number,
): boolean {
  return snapshotVersion === currentVersion
}

export function isDesktopChatSendAvailable(
  runtimeStatus: LocalRuntimeSupervisorState['status'],
  activeProtocol: LocalCodexRuntimeStatus['activeProtocol'] | null,
  hasActiveRun: boolean,
  isSending: boolean,
): boolean {
  return runtimeStatus === 'ready'
    && activeProtocol !== null
    && activeProtocol !== 'unavailable'
    && !hasActiveRun
    && !isSending
}

export function mergeLocalRunEvents(
  current: ReadonlyArray<LocalRunEvent>,
  incoming: ReadonlyArray<LocalRunEvent>,
  limit: number,
): ReadonlyArray<LocalRunEvent> {
  const events = new Map<number, LocalRunEvent>()
  for (const event of current)
    events.set(event.id, event)
  for (const event of incoming)
    events.set(event.id, event)

  return [...events.values()]
    .sort((left, right) => left.id - right.id)
    .slice(-Math.max(1, limit))
}

export function projectDesktopApproval(approval: LocalApproval): DesktopApprovalView {
  const payload = readRecord(approval.payload)
  const params = readRecord(payload?.params)
  const method = readString(payload, 'method')
  const operation = method === 'item/commandExecution/requestApproval'
    ? 'command'
    : method === 'item/fileChange/requestApproval'
      ? 'file_change'
      : 'local'
  const preview = readString(payload, 'promptPreview')
    ?? readString(params, 'command')
    ?? readString(params, 'reason')
    ?? ''

  return {
    approval,
    approvalMode: approval.kind === 'run.codex_app_server_request' ? 'codex' : null,
    authorizationRoot: readString(payload, 'authorizationRoot'),
    cwd: readString(payload, 'cwd'),
    method,
    operation,
    preview,
    scopeReason: readString(payload, 'scopeReason'),
    scopeStatus: readString(payload, 'scopeStatus'),
    targetRoot: readString(payload, 'targetRoot'),
  }
}

function readRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null
}

function draftsMatch(left: DesktopDraftInput, right: DesktopDraftInput): boolean {
  return left.content === right.content
    && JSON.stringify(left.composerContent ?? null) === JSON.stringify(right.composerContent ?? null)
    && left.attachments.length === right.attachments.length
    && left.attachments.every((attachment, index) =>
      attachment.attachmentId === right.attachments[index]?.attachmentId,
    )
}

function readString(record: Record<string, unknown> | null, key: string): string | null {
  const value = record?.[key]
  return typeof value === 'string' && value.trim() ? value.trim() : null
}
