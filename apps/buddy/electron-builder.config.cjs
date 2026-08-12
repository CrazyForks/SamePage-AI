const { join } = require('node:path')
const process = require('node:process')

process.env.SOURCE_DATE_EPOCH ??= '1704067200'

const runtimeExecutables = [
  'lexora-buddy-runtime',
  'lexora-buddy-pet',
]
const macro = name => `$${`{${name}}`}`

module.exports = {
  appId: 'com.lexora.desktop',
  productName: 'Lexora',
  asar: true,
  electronFuses: {
    enableCookieEncryption: true,
    enableNodeCliInspectArguments: false,
    enableNodeOptionsEnvironmentVariable: false,
    onlyLoadAppFromAsar: true,
    runAsNode: false,
  },
  npmRebuild: false,
  directories: {
    output: 'dist-packages',
  },
  files: [
    'out/**/*',
    'package.json',
  ],
  extraResources: [
    ...runtimeExecutables.map(executable => ({
      from: join('runtime', 'target', 'release', executable),
      to: join('runtime', executable),
    })),
    {
      from: join('runtime', 'icons'),
      to: join('runtime', 'icons'),
    },
  ],
  linux: {
    category: 'Utility',
    executableName: 'lexora',
    icon: 'runtime/icons',
    syncDesktopName: true,
    target: ['deb'],
  },
  deb: {
    depends: [
      'libgtk-3-0',
      'libnotify4',
      'libnss3',
      'libxss1',
      'libxtst6',
      'xdg-utils',
      'libatspi2.0-0',
      'libuuid1',
      'libsecret-1-0',
      'libgtk-layer-shell0',
    ],
  },
  artifactName: `Lexora-${macro('version')}-${macro('os')}-${macro('arch')}.${macro('ext')}`,
}
