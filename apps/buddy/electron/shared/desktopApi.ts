import type { LocalChatApi } from './localChatApi'

export const DESKTOP_IPC_CHANNELS = {
  appGetInfo: 'lexora:app:get-info',
  lifecycleQuit: 'lexora:lifecycle:quit',
  settingsGet: 'lexora:settings:get',
  settingsUpdate: 'lexora:settings:update',
  windowGetState: 'lexora:window:get-state',
  windowHide: 'lexora:window:hide',
  windowMinimize: 'lexora:window:minimize',
  windowStateChanged: 'lexora:window:state-changed',
  windowToggleAlwaysOnTop: 'lexora:window:toggle-always-on-top',
  windowToggleMaximize: 'lexora:window:toggle-maximize',
} as const

export interface DesktopWindowState {
  isAlwaysOnTop: boolean
  isMaximized: boolean
}

export interface DesktopAppInfo {
  configPath: string
  version: string
}

export interface LexoraConfig {
  desktop: {
    language: 'zh-CN' | 'en-US'
    launchAtLogin: boolean
    theme: 'system' | 'light' | 'dark'
  }
  agent: {
    codex: {
      defaultModel: string
      reasoningEffort: string
    }
  }
}

export interface LexoraConfigPatch {
  desktop?: Partial<LexoraConfig['desktop']>
  agent?: {
    codex?: Partial<LexoraConfig['agent']['codex']>
  }
}

export interface LexoraDesktopApi {
  app: {
    getInfo: () => Promise<DesktopAppInfo>
  }
  lifecycle: {
    quit: () => Promise<void>
  }
  settings: {
    get: () => Promise<LexoraConfig>
    update: (patch: LexoraConfigPatch) => Promise<LexoraConfig>
  }
  window: {
    getState: () => Promise<DesktopWindowState>
    hide: () => Promise<void>
    minimize: () => Promise<void>
    onStateChanged: (listener: (state: DesktopWindowState) => void) => () => void
    toggleAlwaysOnTop: () => Promise<DesktopWindowState>
    toggleMaximize: () => Promise<DesktopWindowState>
  }
  localChat: LocalChatApi
}
