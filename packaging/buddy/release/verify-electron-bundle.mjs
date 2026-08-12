import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import process from 'node:process'

import { writeOutput } from '../../shared/cli-output.mjs'

const repoRoot = resolve(import.meta.dirname, '../../..')
const bundlePaths = [
  'apps/buddy/out/main/index.js',
  'apps/buddy/out/preload/index.cjs',
]
const forbiddenFragments = [
  'Downloading Electron binary',
  'node_modules/electron/index.js',
  'out/main/install.js',
]

export function verifyElectronBundle(cwd = repoRoot) {
  const errors = []

  for (const relativePath of bundlePaths) {
    const content = readFileSync(resolve(cwd, relativePath), 'utf8')
    if (!content.includes('from "electron"') && !content.includes('require("electron")'))
      errors.push(`${relativePath} does not keep Electron as a runtime external`)

    for (const fragment of forbiddenFragments) {
      if (content.includes(fragment))
        errors.push(`${relativePath} bundled forbidden Electron bootstrap code: ${fragment}`)
    }
  }

  const preload = readFileSync(resolve(cwd, 'apps/buddy/out/preload/index.cjs'), 'utf8')
  if (!preload.includes('require("electron")') || preload.includes('from "electron"'))
    errors.push('apps/buddy/out/preload/index.cjs must be a CommonJS sandbox preload')

  const rendererHtml = readFileSync(resolve(cwd, 'apps/buddy/out/renderer/index.html'), 'utf8')
  if (/connect-src[^;]*(?:localhost|127\.0\.0\.1)/.test(rendererHtml))
    errors.push('apps/buddy/out/renderer/index.html allows development WebSocket origins')

  return errors
}

if (process.argv[1] === new URL(import.meta.url).pathname) {
  const errors = verifyElectronBundle()
  if (errors.length)
    throw new Error(errors.join('\n'))

  writeOutput('Electron bundle boundary check passed')
}
