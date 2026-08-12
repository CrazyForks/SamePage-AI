export type DesktopView = 'chat' | 'agent' | 'settings'
export type DesktopAgentId = 'codex'
export type DesktopSettingsTab = 'general' | 'directories' | 'logs'

export type DesktopRoute
  = | { view: 'chat' }
    | { view: 'agent', agentId: DesktopAgentId | null }
    | { view: 'settings', settingsTab: DesktopSettingsTab }

interface DesktopLocation {
  hash: string
  pathname: string
}

const DESKTOP_SETTINGS_TABS: ReadonlySet<string> = new Set([
  'general',
  'directories',
  'logs',
])

export function resolveDesktopRoute(location: DesktopLocation): DesktopRoute {
  const hashRoute = parseDesktopRoute(location.hash.replace(/^#/, ''))
  if (hashRoute)
    return hashRoute

  const pathRoute = parseDesktopRoute(location.pathname.split('/').filter(Boolean).at(-1)?.replace(/\.html$/, '') ?? '')
  return pathRoute ?? { view: 'chat' }
}

export function toDesktopRouteHash(route: DesktopRoute): string {
  if (route.view === 'settings')
    return `#settings/${route.settingsTab}`
  if (route.view === 'agent')
    return route.agentId ? `#agent/${route.agentId}` : '#agent'

  return '#chat'
}

function parseDesktopRoute(value: string): DesktopRoute | null {
  const [view, detail] = value.split('/')
  if (view === 'chat')
    return { view: 'chat' }
  if (view === 'agent') {
    return {
      agentId: detail === 'codex' ? detail : null,
      view: 'agent',
    }
  }
  if (view === 'settings') {
    return {
      settingsTab: isDesktopSettingsTab(detail) ? detail : 'general',
      view,
    }
  }

  return null
}

function isDesktopSettingsTab(value: string | undefined): value is DesktopSettingsTab {
  return value !== undefined && DESKTOP_SETTINGS_TABS.has(value)
}
