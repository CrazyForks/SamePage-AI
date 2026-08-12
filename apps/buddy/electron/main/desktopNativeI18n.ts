import type { LexoraConfig } from '../shared/desktopApi'
import type { RuntimeSupervisorStatus } from './runtime/RuntimeSupervisor'

type DesktopLanguage = LexoraConfig['desktop']['language']

const messages = {
  'en-US': {
    authorizeProject: 'Authorize local project',
    open: 'Open Lexora',
    quit: 'Quit Lexora',
    restartRuntime: 'Restart local runtime',
    runtime: 'Local runtime: {status}',
    selectAttachments: 'Select attachments',
    statusOffline: 'offline',
    statusReady: 'ready',
    statusRestarting: 'restarting',
    statusStarting: 'starting',
    statusStopped: 'stopped',
    statusStopping: 'stopping',
  },
  'zh-CN': {
    authorizeProject: '授权本地项目',
    open: '打开 Lexora',
    quit: '退出 Lexora',
    restartRuntime: '重新启动本地运行时',
    runtime: '本地运行时：{status}',
    selectAttachments: '选择附件',
    statusOffline: '离线',
    statusReady: '已就绪',
    statusRestarting: '正在重启',
    statusStarting: '正在启动',
    statusStopped: '已停止',
    statusStopping: '正在停止',
  },
} as const

type NativeMessageKey = keyof typeof messages['zh-CN']

export function translateDesktopNative(language: DesktopLanguage, key: NativeMessageKey) {
  return messages[language][key]
}

export function translateDesktopRuntimeStatus(
  language: DesktopLanguage,
  status: RuntimeSupervisorStatus,
) {
  const statusKey = `status${status[0]!.toUpperCase()}${status.slice(1)}` as NativeMessageKey
  return translateDesktopNative(language, 'runtime').replace(
    '{status}',
    translateDesktopNative(language, statusKey),
  )
}
