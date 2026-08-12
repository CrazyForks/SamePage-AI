import type {
  DesktopAppInfo,
  DesktopWindowState,
  LexoraConfigPatch,
  LexoraDesktopApi,
} from '../shared/desktopApi'
import type { LocalChatApi, LocalRunStateEvent, LocalRuntimeSupervisorState } from '../shared/localChatApi'
import { contextBridge, ipcRenderer } from 'electron'
import { DESKTOP_IPC_CHANNELS } from '../shared/desktopApi'
import { LOCAL_CHAT_IPC_CHANNELS } from '../shared/localChatApi'

function subscribe<T>(channel: string, listener: (value: T) => void): () => void {
  const handler = (_event: Electron.IpcRendererEvent, value: T) => listener(value)
  ipcRenderer.on(channel, handler)
  return () => ipcRenderer.off(channel, handler)
}

const localChatApi = Object.freeze<LocalChatApi>({
  runtime: Object.freeze({
    getStatus: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.runtimeStatus),
    getLocalState: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.runtimeLocalState),
    restart: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.runtimeRestart),
    onStateChanged: (listener: (state: LocalRuntimeSupervisorState) => void) =>
      subscribe(LOCAL_CHAT_IPC_CHANNELS.runtimeStateChanged, listener),
  }),
  codex: Object.freeze({
    getStatus: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.codexStatus),
    listModels: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.codexListModels),
    listContextOptions: input =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.codexListContextOptions, input),
  }),
  claude: Object.freeze({
    getStatus: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.claudeStatus),
  }),
  usage: Object.freeze({
    getSnapshot: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.usageSnapshot),
  }),
  projects: Object.freeze({
    authorize: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.projectsAuthorize),
    list: limit => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.projectsList, { limit }),
  }),
  workspaceState: Object.freeze({
    read: () => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.workspaceStateRead),
    write: value => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.workspaceStateWrite, { value }),
  }),
  conversations: Object.freeze({
    list: limit => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.conversationsList, { limit }),
    delete: conversationId =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.conversationsDelete, { conversationId }),
    listMessages: input =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.conversationsListMessages, input),
  }),
  runs: Object.freeze({
    list: input => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.runsList, input ?? {}),
    get: runId => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.runsGet, { runId }),
    listChatEvents: input =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.runsListChatEvents, input),
    listConversationChatEvents: input =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.runsListConversationChatEvents, input),
  }),
  approvals: Object.freeze({
    list: input => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.approvalsList, input ?? {}),
    deny: approvalId =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.approvalsDeny, { approvalId }),
    approveCodex: approvalId =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.approvalsApproveCodex, { approvalId }),
  }),
  attachments: Object.freeze({
    selectFiles: input => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.attachmentsSelectFiles, input),
    release: attachmentIds =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.attachmentsRelease, { attachmentIds }),
    cleanupDrafts: retainedAttachmentIds =>
      ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.attachmentsCleanupDrafts, { retainedAttachmentIds }),
  }),
  chat: Object.freeze({
    startTurn: request => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.chatStartTurn, request),
    cancel: runId => ipcRenderer.invoke(LOCAL_CHAT_IPC_CHANNELS.chatCancel, { runId }),
    onRunEvent: (listener: (event: LocalRunStateEvent) => void) =>
      subscribe(LOCAL_CHAT_IPC_CHANNELS.runEvent, listener),
  }),
})

const desktopApi: LexoraDesktopApi = Object.freeze({
  app: Object.freeze({
    getInfo: (): Promise<DesktopAppInfo> => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.appGetInfo),
  }),
  lifecycle: Object.freeze({
    quit: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.lifecycleQuit),
  }),
  settings: Object.freeze({
    get: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.settingsGet),
    update: (patch: LexoraConfigPatch) => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.settingsUpdate, patch),
  }),
  window: Object.freeze({
    getState: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.windowGetState),
    hide: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.windowHide),
    minimize: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.windowMinimize),
    onStateChanged: (listener: (state: DesktopWindowState) => void) =>
      subscribe(DESKTOP_IPC_CHANNELS.windowStateChanged, listener),
    toggleAlwaysOnTop: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.windowToggleAlwaysOnTop),
    toggleMaximize: () => ipcRenderer.invoke(DESKTOP_IPC_CHANNELS.windowToggleMaximize),
  }),
  localChat: localChatApi,
})

contextBridge.exposeInMainWorld('lexoraDesktop', desktopApi)
