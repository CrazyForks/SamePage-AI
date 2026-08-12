export function isAllowedRendererNavigation(targetUrl: string, rendererUrl: string): boolean {
  const target = parseUrl(targetUrl)
  const renderer = parseUrl(rendererUrl)
  if (!target || !renderer)
    return false

  return target.protocol === renderer.protocol
    && target.host === renderer.host
    && target.pathname === renderer.pathname
    && target.search === renderer.search
}

export function isAllowedExternalUrl(targetUrl: string): boolean {
  return parseUrl(targetUrl)?.protocol === 'https:'
}

export function resolveDevelopmentRendererUrl(
  rendererUrl: string | undefined,
  isPackaged: boolean,
): string | null {
  if (isPackaged || !rendererUrl)
    return null

  const url = parseUrl(rendererUrl)
  if (!url || url.protocol !== 'http:' || !isLoopbackHostname(url.hostname))
    return null

  return url.toString()
}

function isLoopbackHostname(hostname: string): boolean {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]'
}

function parseUrl(value: string): URL | null {
  try {
    return new URL(value)
  }
  catch {
    return null
  }
}
