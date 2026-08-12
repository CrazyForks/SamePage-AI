import type {
  LocalApproval,
  LocalAttachment,
  LocalClaudeRuntimeStatus,
  LocalCodexContextOptions,
  LocalCodexRuntimeStatus,
  LocalConversation,
  LocalMessage,
  LocalProject,
  LocalRun,
  LocalRunEvent,
  LocalRunStateEvent,
  LocalRuntimeModelOption,
  LocalRuntimeSupervisorState,
  LocalStartTurnRequest,
  LocalStateStatus,
  LocalTurnStart,
  LocalUsageSnapshot,
  LocalWorkspaceSetting,
  LocalWorkspaceStateValue,
} from './localChatApiSchemas'

export type LocalChatErrorCode
  = | 'CODEX_RUNTIME_FAILED'
    | 'LOCAL_CHAT_OPERATION_FAILED'
    | 'LOCAL_DATA_INVALID'
    | 'LOCAL_IO_FAILED'
    | 'LOCAL_STORAGE_FAILED'
    | 'RUNTIME_EXECUTION_FAILED'
    | 'RUNTIME_PROTOCOL_ERROR'
    | 'RUNTIME_RESPONSE_INVALID'
    | 'RUNTIME_UNAVAILABLE'
    | 'UNSUPPORTED_CAPABILITY'
    | 'VALIDATION_FAILED'

export interface LocalChatPublicError {
  code: LocalChatErrorCode
  retryable: boolean
}

const LOCAL_CHAT_ERROR_MARKER = 'LEXORA_LOCAL_CHAT_ERROR'
const LOCAL_CHAT_ERROR_PATTERN = /LEXORA_LOCAL_CHAT_ERROR:([A-Z0-9_]+):(0|1)/

export function formatLocalChatPublicError(error: LocalChatPublicError): string {
  return `${LOCAL_CHAT_ERROR_MARKER}:${error.code}:${error.retryable ? '1' : '0'}`
}

export function parseLocalChatPublicError(message: string): LocalChatPublicError | null {
  const match = LOCAL_CHAT_ERROR_PATTERN.exec(message)
  if (!match || !isLocalChatErrorCode(match[1]))
    return null

  return {
    code: match[1],
    retryable: match[2] === '1',
  }
}

export function isLocalChatErrorCode(value: string | undefined): value is LocalChatErrorCode {
  return value === 'CODEX_RUNTIME_FAILED'
    || value === 'LOCAL_CHAT_OPERATION_FAILED'
    || value === 'LOCAL_DATA_INVALID'
    || value === 'LOCAL_IO_FAILED'
    || value === 'LOCAL_STORAGE_FAILED'
    || value === 'RUNTIME_EXECUTION_FAILED'
    || value === 'RUNTIME_PROTOCOL_ERROR'
    || value === 'RUNTIME_RESPONSE_INVALID'
    || value === 'RUNTIME_UNAVAILABLE'
    || value === 'UNSUPPORTED_CAPABILITY'
    || value === 'VALIDATION_FAILED'
}

export type {
  LocalApproval,
  LocalAttachment,
  LocalClaudeRuntimeStatus,
  LocalCodexContextOptions,
  LocalCodexInput,
  LocalCodexRuntimeStatus,
  LocalConversation,
  LocalMessage,
  LocalMessageAttachment,
  LocalProject,
  LocalPromptContextOption,
  LocalRun,
  LocalRunEvent,
  LocalRunStateEvent,
  LocalRuntimeModelOption,
  LocalRuntimeSupervisorState,
  LocalStartTurnRequest,
  LocalStateStatus,
  LocalTurnStart,
  LocalUsageSnapshot,
  LocalWorkspaceDraft,
  LocalWorkspaceSetting,
  LocalWorkspaceStateValue,
} from './localChatApiSchemas'

export const LOCAL_CHAT_IPC_CHANNELS = {
  runtimeStatus: 'lexora:runtime:status',
  runtimeLocalState: 'lexora:runtime:local-state',
  runtimeRestart: 'lexora:runtime:restart',
  runtimeStateChanged: 'lexora:runtime:state-changed',
  runEvent: 'lexora:chat:run-event',
  codexStatus: 'lexora:codex:status',
  codexListModels: 'lexora:codex:list-models',
  codexListContextOptions: 'lexora:codex:list-context-options',
  claudeStatus: 'lexora:claude:status',
  usageSnapshot: 'lexora:usage:snapshot',
  projectsAuthorize: 'lexora:projects:authorize',
  projectsList: 'lexora:projects:list',
  workspaceStateRead: 'lexora:workspace-state:read',
  workspaceStateWrite: 'lexora:workspace-state:write',
  conversationsList: 'lexora:conversations:list',
  conversationsDelete: 'lexora:conversations:delete',
  conversationsListMessages: 'lexora:conversations:list-messages',
  runsList: 'lexora:runs:list',
  runsGet: 'lexora:runs:get',
  runsListChatEvents: 'lexora:runs:list-chat-events',
  runsListConversationChatEvents: 'lexora:runs:list-conversation-chat-events',
  approvalsList: 'lexora:approvals:list',
  approvalsDeny: 'lexora:approvals:deny',
  approvalsApproveCodex: 'lexora:approvals:approve-codex',
  attachmentsSelectFiles: 'lexora:attachments:select-files',
  attachmentsRelease: 'lexora:attachments:release',
  attachmentsCleanupDrafts: 'lexora:attachments:cleanup-drafts',
  chatStartTurn: 'lexora:chat:start-turn',
  chatCancel: 'lexora:chat:cancel',
} as const

export interface LocalChatApi {
  runtime: {
    getStatus: () => Promise<LocalRuntimeSupervisorState>
    getLocalState: () => Promise<LocalStateStatus>
    restart: () => Promise<LocalRuntimeSupervisorState>
    onStateChanged: (listener: (state: LocalRuntimeSupervisorState) => void) => () => void
  }
  codex: {
    getStatus: () => Promise<LocalCodexRuntimeStatus>
    listModels: () => Promise<ReadonlyArray<LocalRuntimeModelOption>>
    listContextOptions: (input: {
      cwd: string | null
      fileQuery?: string | null
    }) => Promise<LocalCodexContextOptions>
  }
  claude: {
    getStatus: () => Promise<LocalClaudeRuntimeStatus>
  }
  usage: {
    getSnapshot: () => Promise<LocalUsageSnapshot>
  }
  projects: {
    authorize: () => Promise<LocalProject | null>
    list: (limit?: number) => Promise<ReadonlyArray<LocalProject>>
  }
  workspaceState: {
    read: () => Promise<LocalWorkspaceSetting | null>
    write: (value: LocalWorkspaceStateValue) => Promise<LocalWorkspaceSetting>
  }
  conversations: {
    list: (limit?: number) => Promise<ReadonlyArray<LocalConversation>>
    delete: (conversationId: string) => Promise<boolean>
    listMessages: (input: {
      conversationId: string
      limit?: number
    }) => Promise<ReadonlyArray<LocalMessage>>
  }
  runs: {
    list: (input?: {
      sessionId?: string | null
      conversationId?: string | null
      limit?: number
    }) => Promise<ReadonlyArray<LocalRun>>
    get: (runId: string) => Promise<LocalRun>
    listChatEvents: (input: {
      runId: string
      afterId?: number | null
      limit?: number
    }) => Promise<ReadonlyArray<LocalRunEvent>>
    listConversationChatEvents: (input: {
      conversationId: string
      afterId?: number | null
      runLimit?: number
      eventLimit?: number
    }) => Promise<ReadonlyArray<LocalRunEvent>>
  }
  approvals: {
    list: (input?: { status?: string | null, limit?: number }) => Promise<ReadonlyArray<LocalApproval>>
    deny: (approvalId: string) => Promise<unknown>
    approveCodex: (approvalId: string) => Promise<unknown>
  }
  attachments: {
    selectFiles: (input: { remainingCount: number }) => Promise<ReadonlyArray<LocalAttachment>>
    release: (attachmentIds: ReadonlyArray<string>) => Promise<{ releasedAttachmentIds: ReadonlyArray<string> }>
    cleanupDrafts: (retainedAttachmentIds: ReadonlyArray<string>) => Promise<{ releasedAttachmentIds: ReadonlyArray<string> }>
  }
  chat: {
    startTurn: (request: LocalStartTurnRequest) => Promise<LocalTurnStart>
    cancel: (runId: string) => Promise<LocalRun>
    onRunEvent: (listener: (event: LocalRunStateEvent) => void) => () => void
  }
}
