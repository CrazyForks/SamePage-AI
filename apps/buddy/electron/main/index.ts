import type { BrowserWindow } from 'electron'
import type { LexoraConfig } from '../shared/desktopApi'
import { join } from 'node:path'
import process from 'node:process'
import { app, Menu, nativeTheme } from 'electron'
import { installAttachmentProtocol, registerAttachmentSchemePrivileges } from './attachmentProtocol'
import { LexoraConfigStore } from './config/LexoraConfigStore'
import { resolveDesktopIconPath } from './desktopIcon'
import { registerDesktopIpc } from './ipc'
import { resolveLinuxConfigDirectory, syncLinuxAutostart } from './linuxAutostart'
import { registerLocalChatIpc } from './localChatIpc'
import { resolveLexoraHome } from './paths'
import { createRuntimeProcessFactory } from './runtime/runtimeProcess'
import { RuntimeSupervisor } from './runtime/RuntimeSupervisor'
import { resolveDevelopmentRendererUrl } from './security/navigationPolicy'
import { createDesktopTray } from './tray'
import { createDesktopWindow } from './window'

let desktopWindow: BrowserWindow | null = null
let runtimeSupervisor: RuntimeSupervisor | null = null
let stopLocalChatIpc: (() => void) | null = null
let stopRuntimeNotification: (() => void) | null = null
let stopRuntimeStateSubscription: (() => void) | null = null
let stopAttachmentProtocol: (() => void) | null = null
let desktopTray: ReturnType<typeof createDesktopTray> | null = null
let isQuitting = false
let quitCommitted = false
let quitPromise: Promise<void> | null = null
let desktopLanguage: LexoraConfig['desktop']['language'] = 'zh-CN'

registerAttachmentSchemePrivileges()
if (process.platform === 'linux')
  app.setDesktopName('lexora-buddy')

if (!app.requestSingleInstanceLock()) {
  app.quit()
}
else {
  app.on('before-quit', (event) => {
    isQuitting = true
    if (quitCommitted)
      return

    event.preventDefault()
    void quitLexora()
  })

  app.on('second-instance', () => {
    showDesktopWindow()
  })

  app.on('activate', () => {
    showDesktopWindow()
  })

  app.on('window-all-closed', () => {})

  void app.whenReady().then(async () => {
    app.setName('Lexora')
    app.setAppUserModelId('com.lexora.desktop')
    Menu.setApplicationMenu(null)

    const isSmokeTest = process.env.LEXORA_DESKTOP_SMOKE_TEST === '1'
    const lexoraHome = resolveLexoraHome()
    const configPath = join(lexoraHome, 'config.toml')
    const configStore = new LexoraConfigStore({ configPath })
    await applyDesktopConfig(await configStore.read())
    runtimeSupervisor = new RuntimeSupervisor({
      spawnRuntime: createRuntimeProcessFactory({
        appPath: app.getAppPath(),
        env: {
          ...process.env,
          LEXORA_HOME: lexoraHome,
        },
        isPackaged: app.isPackaged,
        resourcesPath: process.resourcesPath,
        runtimePathOverride: process.env.LEXORA_BUDDY_RUNTIME_PATH,
      }),
    })
    runtimeSupervisor.start()
    stopAttachmentProtocol = installAttachmentProtocol(runtimeSupervisor)

    desktopTray = createDesktopTray({
      appPath: app.getAppPath(),
      isPackaged: app.isPackaged,
      language: desktopLanguage,
      onOpenDesktop: showDesktopWindow,
      onQuit() {
        void quitLexora()
      },
      resourcesPath: process.resourcesPath,
      runtime: runtimeSupervisor,
    })
    stopRuntimeStateSubscription = runtimeSupervisor.onStateChange((state) => {
      desktopTray?.setRuntimeState(state)
    })
    stopRuntimeNotification = runtimeSupervisor.onNotification((notification) => {
      if (notification.method === 'desktop.open')
        showDesktopWindow()
    })

    registerDesktopIpc({
      configPath,
      configStore,
      getWindow: () => desktopWindow,
      onConfigUpdated: applyDesktopConfig,
      requestQuit() {
        void quitLexora()
      },
    })
    stopLocalChatIpc = registerLocalChatIpc({
      getLanguage: () => desktopLanguage,
      getWindow: () => desktopWindow,
      runtime: runtimeSupervisor,
    })

    const desktop = createDesktopWindow({
      iconPath: resolveDesktopIconPath({
        appPath: app.getAppPath(),
        isPackaged: app.isPackaged,
        resourcesPath: process.resourcesPath,
      }),
      isQuitting: () => isQuitting,
      rendererUrl: resolveDevelopmentRendererUrl(
        process.env.ELECTRON_RENDERER_URL,
        app.isPackaged,
      ),
      showOnReady: !isSmokeTest,
    })
    desktopWindow = desktop.window

    desktopWindow.on('closed', () => {
      desktopWindow = null
    })
    await desktop.load()

    if (isSmokeTest) {
      const bridgeAvailable = await desktop.window.webContents.executeJavaScript(
        'typeof globalThis.lexoraDesktop === "object"',
        true,
      )
      if (bridgeAvailable !== true)
        throw new Error('Lexora Desktop Preload bridge is unavailable')

      const codexStatus = await desktop.window.webContents.executeJavaScript(
        'globalThis.lexoraDesktop.localChat.codex.getStatus()',
        true,
      )
      if (!codexStatus || typeof codexStatus !== 'object' || typeof codexStatus.cliAvailable !== 'boolean')
        throw new Error('Lexora Desktop Preload local chat IPC is unavailable')

      await quitLexora()
    }
  }).catch(async (error) => {
    console.error('Lexora Desktop failed to start', error)
    isQuitting = true
    stopLocalChatIpc?.()
    stopAttachmentProtocol?.()
    stopRuntimeNotification?.()
    stopRuntimeStateSubscription?.()
    await runtimeSupervisor?.stop()
    desktopTray?.destroy()
    app.exit(1)
  })
}

function quitLexora(): Promise<void> {
  isQuitting = true
  if (quitPromise)
    return quitPromise

  quitPromise = (async () => {
    stopLocalChatIpc?.()
    stopLocalChatIpc = null
    stopRuntimeNotification?.()
    stopRuntimeNotification = null
    stopRuntimeStateSubscription?.()
    stopRuntimeStateSubscription = null
    stopAttachmentProtocol?.()
    stopAttachmentProtocol = null
    await runtimeSupervisor?.stop()
    desktopTray?.destroy()
    desktopTray = null
    quitCommitted = true
    app.quit()
  })()

  return quitPromise
}

function showDesktopWindow(): void {
  if (!desktopWindow)
    return

  if (desktopWindow.isMinimized())
    desktopWindow.restore()

  desktopWindow.show()
  desktopWindow.focus()
}

async function applyDesktopConfig(config: LexoraConfig): Promise<void> {
  desktopLanguage = config.desktop.language
  desktopTray?.setLanguage(desktopLanguage)
  nativeTheme.themeSource = config.desktop.theme
  if (!app.isPackaged || process.env.LEXORA_DESKTOP_SMOKE_TEST === '1')
    return

  if (process.platform === 'linux') {
    await syncLinuxAutostart({
      configDirectory: resolveLinuxConfigDirectory(
        app.getPath('home'),
        process.env.XDG_CONFIG_HOME,
      ),
      enabled: config.desktop.launchAtLogin,
      executablePath: process.execPath,
    })
    return
  }

  app.setLoginItemSettings({
    openAtLogin: config.desktop.launchAtLogin,
  })
}
