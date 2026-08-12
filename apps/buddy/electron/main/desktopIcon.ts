import { existsSync } from 'node:fs'
import { join } from 'node:path'

export interface DesktopIconOptions {
  appPath: string
  isPackaged: boolean
  resourcesPath: string
}

export function resolveDesktopIconPath(options: DesktopIconOptions): string {
  const iconPath = options.isPackaged
    ? join(options.resourcesPath, 'runtime', 'icons', 'icon.png')
    : join(options.appPath, 'runtime', 'icons', 'icon.png')

  if (!existsSync(iconPath))
    throw new Error(`Lexora desktop icon is missing: ${iconPath}`)

  return iconPath
}
