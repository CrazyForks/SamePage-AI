import type { BrowserWindowConstructorOptions } from 'electron'
import type { DesktopWindowState } from '../shared/desktopApi'
import { join } from 'node:path'
import { BrowserWindow, shell } from 'electron'
import { DESKTOP_IPC_CHANNELS } from '../shared/desktopApi'
import { isAllowedExternalUrl, isAllowedRendererNavigation } from './security/navigationPolicy'

export interface CreateDesktopWindowOptions {
  iconPath: string
  isQuitting: () => boolean
  rendererUrl: string | null
  showOnReady?: boolean
}

export interface DesktopWindowHandle {
  load: () => Promise<void>
  window: BrowserWindow
}

export function createDesktopWindow(options: CreateDesktopWindowOptions): DesktopWindowHandle {
  const windowOptions: BrowserWindowConstructorOptions = {
    width: 1280,
    height: 820,
    minWidth: 980,
    minHeight: 640,
    autoHideMenuBar: true,
    frame: false,
    icon: options.iconPath,
    show: false,
    title: 'Lexora',
    webPreferences: {
      preload: join(__dirname, '../preload/index.cjs'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      webSecurity: true,
      allowRunningInsecureContent: false,
    },
  }
  const window = new BrowserWindow(windowOptions)
  window.removeMenu()
  let trustedRendererUrl = ''

  const publishWindowState = () => {
    if (!window.webContents.isDestroyed()) {
      window.webContents.send(
        DESKTOP_IPC_CHANNELS.windowStateChanged,
        readDesktopWindowState(window),
      )
    }
  }

  window.on('always-on-top-changed', publishWindowState)
  window.on('maximize', publishWindowState)
  window.on('unmaximize', publishWindowState)

  window.on('close', (event) => {
    if (options.isQuitting())
      return

    event.preventDefault()
    window.hide()
  })

  window.once('ready-to-show', () => {
    if (options.showOnReady !== false)
      window.show()
  })

  window.webContents.setWindowOpenHandler(({ url }) => {
    if (isAllowedExternalUrl(url))
      void shell.openExternal(url)

    return { action: 'deny' }
  })

  window.webContents.on('will-navigate', (event, url) => {
    if (isAllowedRendererNavigation(url, trustedRendererUrl))
      return

    event.preventDefault()
    if (isAllowedExternalUrl(url))
      void shell.openExternal(url)
  })

  return {
    window,
    async load() {
      if (options.rendererUrl) {
        trustedRendererUrl = new URL('/chat', options.rendererUrl).toString()
        await window.loadURL(trustedRendererUrl)
        return
      }

      const rendererPath = join(__dirname, '../renderer/index.html')
      await window.loadFile(rendererPath, { hash: 'chat' })
      trustedRendererUrl = window.webContents.getURL()
    },
  }
}

export function readDesktopWindowState(window: BrowserWindow): DesktopWindowState {
  return {
    isAlwaysOnTop: window.isAlwaysOnTop(),
    isMaximized: window.isMaximized(),
  }
}
