import type { BrowserWindow, IpcMainInvokeEvent, OpenDialogOptions } from 'electron'
import type { ZodType } from 'zod'
import type { LexoraConfig } from '../shared/desktopApi'
import type { LocalChatErrorCode } from '../shared/localChatApi'
import type { RuntimeRequestOptions } from './runtime/RuntimeRpcClient'
import type { RuntimeSupervisor } from './runtime/RuntimeSupervisor'
import { dialog, ipcMain } from 'electron'
import { ZodError } from 'zod'
import {
  formatLocalChatPublicError,
  isLocalChatErrorCode,
  LOCAL_CHAT_IPC_CHANNELS,
} from '../shared/localChatApi'
import {
  LOCAL_WORKSPACE_STATE_KEY,
  localChatResponseSchemas,
  localChatSchemas,
} from '../shared/localChatApiSchemas'
import { translateDesktopNative } from './desktopNativeI18n'
import { assertTrustedSender } from './ipc'
import {
  RuntimeProtocolError,
  RuntimeRequestError,
  RuntimeUnavailableError,
} from './runtime/RuntimeRpcClient'

const USAGE_REQUEST_TIMEOUT_MS = 120_000

export interface RegisterLocalChatIpcOptions {
  getWindow: () => BrowserWindow | null
  getLanguage: () => LexoraConfig['desktop']['language']
  runtime: RuntimeSupervisor
}

export function registerLocalChatIpc(options: RegisterLocalChatIpcOptions): () => void {
  const handle = <T>(
    channel: string,
    handler: (event: IpcMainInvokeEvent, input: T) => unknown,
  ) => {
    ipcMain.handle(channel, async (event, input: T) => {
      try {
        assertTrustedSender(event, options.getWindow())
        return await handler(event, input)
      }
      catch (error) {
        throw createLocalChatIpcError(error)
      }
    })
  }
  const request = async <T>(
    method: string,
    params: unknown,
    schema: ZodType<T>,
    requestOptions?: RuntimeRequestOptions,
  ): Promise<T> => {
    const result = schema.safeParse(await options.runtime.request(method, params, requestOptions))
    if (!result.success)
      throw new RuntimeProtocolError(`Lexora Runtime returned an invalid response for ${method}`)

    return result.data
  }

  handle(LOCAL_CHAT_IPC_CHANNELS.runtimeStatus, () => {
    return localChatResponseSchemas.runtimeState.parse(options.runtime.state)
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.runtimeLocalState, () => {
    return request('runtime.localState', {}, localChatResponseSchemas.localState)
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.runtimeRestart, async () => {
    await options.runtime.restart()
    return localChatResponseSchemas.runtimeState.parse(options.runtime.state)
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.codexStatus, () => {
    return request('codex.status', {}, localChatResponseSchemas.codexStatus)
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.codexListModels, () => {
    return request('codex.listModels', {}, localChatResponseSchemas.modelOptions)
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.codexListContextOptions, (_event, input) => {
    return request(
      'codex.listContextOptions',
      localChatSchemas.codexContext.parse(input),
      localChatResponseSchemas.contextOptions,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.claudeStatus, () => {
    return request('claude.status', {}, localChatResponseSchemas.claudeStatus)
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.usageSnapshot, () => {
    return request(
      'usage.snapshot',
      {},
      localChatResponseSchemas.usageSnapshot,
      { timeoutMs: USAGE_REQUEST_TIMEOUT_MS },
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.projectsAuthorize, async () => {
    const paths = await selectPaths(options.getWindow(), {
      properties: ['openDirectory'],
      title: translateDesktopNative(options.getLanguage(), 'authorizeProject'),
    })
    const root = paths[0]
    if (!root)
      return null

    return request(
      'projects.authorize',
      { root, name: null },
      localChatResponseSchemas.project,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.projectsList, (_event, input) => {
    return request(
      'projects.list',
      localChatSchemas.limit.parse(input),
      localChatResponseSchemas.projects,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.workspaceStateRead, () => {
    return request(
      'workspaceState.read',
      { key: LOCAL_WORKSPACE_STATE_KEY },
      localChatResponseSchemas.optionalWorkspaceSetting,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.workspaceStateWrite, (_event, input) => {
    const { value } = localChatSchemas.workspaceValue.parse(input)
    return request(
      'workspaceState.write',
      { key: LOCAL_WORKSPACE_STATE_KEY, value },
      localChatResponseSchemas.workspaceSetting,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.conversationsList, (_event, input) => {
    return request(
      'conversations.list',
      localChatSchemas.limit.parse(input),
      localChatResponseSchemas.conversations,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.conversationsDelete, (_event, input) => {
    return request(
      'conversations.delete',
      localChatSchemas.conversationId.parse(input),
      localChatResponseSchemas.deleted,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.conversationsListMessages, (_event, input) => {
    return request(
      'conversations.listMessages',
      localChatSchemas.conversationMessages.parse(input),
      localChatResponseSchemas.messages,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.runsList, (_event, input) => {
    return request(
      'runs.list',
      localChatSchemas.listRuns.parse(input),
      localChatResponseSchemas.runs,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.runsGet, (_event, input) => {
    return request('runs.get', localChatSchemas.runId.parse(input), localChatResponseSchemas.run)
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.runsListChatEvents, (_event, input) => {
    return request(
      'runs.listChatEvents',
      localChatSchemas.runEvents.parse(input),
      localChatResponseSchemas.runEvents,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.runsListConversationChatEvents, (_event, input) => {
    return request(
      'runs.listConversationChatEvents',
      localChatSchemas.conversationEvents.parse(input),
      localChatResponseSchemas.runEvents,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.approvalsList, (_event, input) => {
    return request(
      'approvals.list',
      localChatSchemas.listApprovals.parse(input),
      localChatResponseSchemas.approvals,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.approvalsDeny, (_event, input) => {
    return request(
      'approvals.deny',
      localChatSchemas.approvalId.parse(input),
      localChatResponseSchemas.approvalResolution,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.approvalsApproveCodex, (_event, input) => {
    return request(
      'approvals.approveCodex',
      localChatSchemas.approvalId.parse(input),
      localChatResponseSchemas.approvalResolution,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.attachmentsSelectFiles, async (_event, input) => {
    const { remainingCount } = localChatSchemas.attachmentSelection.parse(input)
    const paths = await selectPaths(options.getWindow(), {
      properties: ['openFile', 'multiSelections'],
      title: translateDesktopNative(options.getLanguage(), 'selectAttachments'),
    })
    if (paths.length === 0)
      return []

    return request(
      'attachments.registerFiles',
      { paths: paths.slice(0, remainingCount) },
      localChatResponseSchemas.attachments,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.attachmentsRelease, (_event, input) => {
    return request(
      'attachments.release',
      localChatSchemas.attachmentRelease.parse(input),
      localChatResponseSchemas.releasedAttachments,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.attachmentsCleanupDrafts, (_event, input) => {
    return request(
      'attachments.cleanupDrafts',
      localChatSchemas.retainedAttachments.parse(input),
      localChatResponseSchemas.releasedAttachments,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.chatStartTurn, (_event, input) => {
    return request(
      'chat.startTurn',
      localChatSchemas.startTurn.parse(input),
      localChatResponseSchemas.turnStart,
    )
  })
  handle(LOCAL_CHAT_IPC_CHANNELS.chatCancel, (_event, input) => {
    return request(
      'chat.cancel',
      localChatSchemas.runId.parse(input),
      localChatResponseSchemas.run,
    )
  })

  const stopStateSubscription = options.runtime.onStateChange((state) => {
    sendToRenderer(options.getWindow(), LOCAL_CHAT_IPC_CHANNELS.runtimeStateChanged, state)
  })
  const stopNotificationSubscription = options.runtime.onNotification((notification) => {
    if (notification.method !== 'run.event')
      return

    const event = localChatSchemas.runStateEvent.safeParse(notification.params)
    if (event.success)
      sendToRenderer(options.getWindow(), LOCAL_CHAT_IPC_CHANNELS.runEvent, event.data)
  })

  return () => {
    stopNotificationSubscription()
    stopStateSubscription()
  }
}

function createLocalChatIpcError(error: unknown): Error {
  if (error instanceof RuntimeRequestError) {
    const runtimeError = readRuntimePublicError(error.data)
    if (runtimeError)
      return new Error(formatLocalChatPublicError(runtimeError))
  }
  if (error instanceof RuntimeUnavailableError) {
    return new Error(formatLocalChatPublicError({
      code: 'RUNTIME_UNAVAILABLE',
      retryable: true,
    }))
  }
  if (error instanceof RuntimeProtocolError) {
    return new Error(formatLocalChatPublicError({
      code: 'RUNTIME_PROTOCOL_ERROR',
      retryable: false,
    }))
  }
  if (error instanceof ZodError) {
    return new Error(formatLocalChatPublicError({
      code: 'VALIDATION_FAILED',
      retryable: false,
    }))
  }

  return new Error(formatLocalChatPublicError({
    code: 'LOCAL_CHAT_OPERATION_FAILED',
    retryable: false,
  }))
}

function readRuntimePublicError(value: unknown): { code: LocalChatErrorCode, retryable: boolean } | null {
  if (
    !isRecord(value)
    || typeof value.code !== 'string'
    || !isLocalChatErrorCode(value.code)
    || typeof value.retryable !== 'boolean'
  ) {
    return null
  }

  return { code: value.code, retryable: value.retryable }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function selectPaths(
  window: BrowserWindow | null,
  options: OpenDialogOptions,
): Promise<string[]> {
  const result = window
    ? await dialog.showOpenDialog(window, options)
    : await dialog.showOpenDialog(options)

  return result.canceled ? [] : result.filePaths
}

function sendToRenderer(window: BrowserWindow | null, channel: string, payload: unknown): void {
  if (!window || window.isDestroyed() || window.webContents.isDestroyed())
    return

  window.webContents.send(channel, payload)
}
