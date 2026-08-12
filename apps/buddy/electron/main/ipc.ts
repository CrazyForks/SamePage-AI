import type { BrowserWindow, IpcMainInvokeEvent } from 'electron'
import type { LexoraConfig } from '../shared/desktopApi'
import type { LexoraConfigStore } from './config/LexoraConfigStore'
import { app, ipcMain } from 'electron'
import {
  DESKTOP_IPC_CHANNELS,
} from '../shared/desktopApi'
import { lexoraConfigPatchSchema } from '../shared/desktopApiSchemas'
import { readDesktopWindowState } from './window'

export interface RegisterDesktopIpcOptions {
  configPath: string
  configStore: LexoraConfigStore
  getWindow: () => BrowserWindow | null
  onConfigUpdated: (config: LexoraConfig) => Promise<void> | void
  requestQuit: () => void
}

export function registerDesktopIpc(options: RegisterDesktopIpcOptions): void {
  ipcMain.handle(DESKTOP_IPC_CHANNELS.appGetInfo, (event) => {
    assertTrustedSender(event, options.getWindow())
    return {
      configPath: options.configPath,
      version: app.getVersion(),
    }
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.lifecycleQuit, (event) => {
    assertTrustedSender(event, options.getWindow())
    setImmediate(options.requestQuit)
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.settingsGet, (event) => {
    assertTrustedSender(event, options.getWindow())
    return options.configStore.read()
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.settingsUpdate, async (event, input: unknown) => {
    assertTrustedSender(event, options.getWindow())
    const config = await options.configStore.update(lexoraConfigPatchSchema.parse(input))
    await options.onConfigUpdated(config)
    return config
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.windowGetState, (event) => {
    const window = requireTrustedWindow(event, options.getWindow())
    return readDesktopWindowState(window)
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.windowHide, (event) => {
    requireTrustedWindow(event, options.getWindow()).hide()
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.windowMinimize, (event) => {
    requireTrustedWindow(event, options.getWindow()).minimize()
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.windowToggleAlwaysOnTop, (event) => {
    const window = requireTrustedWindow(event, options.getWindow())
    window.setAlwaysOnTop(!window.isAlwaysOnTop())
    return readDesktopWindowState(window)
  })

  ipcMain.handle(DESKTOP_IPC_CHANNELS.windowToggleMaximize, (event) => {
    const window = requireTrustedWindow(event, options.getWindow())
    if (window.isMaximized())
      window.unmaximize()
    else
      window.maximize()

    return readDesktopWindowState(window)
  })
}

function requireTrustedWindow(
  event: IpcMainInvokeEvent,
  window: BrowserWindow | null,
): BrowserWindow {
  assertTrustedSender(event, window)
  return window as BrowserWindow
}

export function assertTrustedSender(event: IpcMainInvokeEvent, window: BrowserWindow | null): void {
  if (!window
    || event.sender !== window.webContents
    || event.senderFrame !== window.webContents.mainFrame) {
    throw new Error('Untrusted Desktop IPC sender')
  }
}
