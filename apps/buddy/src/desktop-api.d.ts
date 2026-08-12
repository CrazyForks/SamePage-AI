import type { LexoraDesktopApi } from '../electron/shared/desktopApi'

declare global {
  interface Window {
    lexoraDesktop?: LexoraDesktopApi
  }
}

export {}
