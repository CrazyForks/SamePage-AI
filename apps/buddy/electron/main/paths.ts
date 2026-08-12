import { homedir } from 'node:os'
import { isAbsolute, join, normalize } from 'node:path'
import process from 'node:process'

export function resolveLexoraHome(
  override = process.env.LEXORA_HOME,
  userHome = homedir(),
): string {
  if (!override)
    return join(userHome, '.lexora')

  if (!isAbsolute(override))
    throw new Error('LEXORA_HOME must be an absolute path')

  return normalize(override)
}
