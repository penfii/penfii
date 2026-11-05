/**
 * Where to go after signing in. An OAuth consent link (`/consent?request=…`) must survive the
 * login page, including a full-page provider round trip, so it is kept for the tab only.
 */
const KEY = 'penfii.afterLogin'

export function rememberReturn(path: string): void {
  if (!path.startsWith('/') || path.startsWith('//') || path === '/' || path.startsWith('/login')) return
  try {
    sessionStorage.setItem(KEY, path)
  } catch {
    // Storage can be blocked; the person lands on the home page instead.
  }
}

/** The remembered path once, or `fallback`. Only same-site relative paths are ever returned. */
export function takeReturn(fallback = '/'): string {
  try {
    const stored = sessionStorage.getItem(KEY)
    sessionStorage.removeItem(KEY)
    if (stored?.startsWith('/') && !stored.startsWith('//')) return stored
  } catch {
    // ignore
  }
  return fallback
}

export function peekReturn(): string | undefined {
  try {
    const stored = sessionStorage.getItem(KEY)
    return stored?.startsWith('/') && !stored.startsWith('//') ? stored : undefined
  } catch {
    return undefined
  }
}
