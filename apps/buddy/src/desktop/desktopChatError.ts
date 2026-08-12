import type { BuddyLocale } from '@/i18n/buddyI18n'
import { translateBuddy } from '@/i18n/buddyI18n'
import { parseLocalChatPublicError } from '../../electron/shared/localChatApi'

export function normalizeDesktopError(error: unknown, language: BuddyLocale): string {
  if (error instanceof Error) {
    const publicError = parseLocalChatPublicError(error.message)
    if (publicError) {
      switch (publicError.code) {
        case 'VALIDATION_FAILED':
          return translateBuddy(language, 'desktop.error.validation')
        case 'UNSUPPORTED_CAPABILITY':
          return translateBuddy(language, 'desktop.error.unsupportedCapability')
        case 'CODEX_RUNTIME_FAILED':
          return translateBuddy(language, 'desktop.error.codexRuntime')
        case 'LOCAL_IO_FAILED':
          return translateBuddy(language, 'desktop.error.localIo')
        case 'LOCAL_STORAGE_FAILED':
          return translateBuddy(language, 'desktop.error.localStorage')
        case 'LOCAL_DATA_INVALID':
          return translateBuddy(language, 'desktop.error.localData')
        case 'RUNTIME_UNAVAILABLE':
          return translateBuddy(language, 'desktop.error.runtimeUnavailable')
        case 'RUNTIME_PROTOCOL_ERROR':
        case 'RUNTIME_RESPONSE_INVALID':
          return translateBuddy(language, 'desktop.error.runtimeProtocol')
        case 'RUNTIME_EXECUTION_FAILED':
        case 'LOCAL_CHAT_OPERATION_FAILED':
          return translateBuddy(language, 'desktop.chat.unknownError')
      }
    }
  }

  return translateBuddy(language, 'desktop.chat.unknownError')
}
