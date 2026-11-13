/**
 * Where to go after signing in. An OAuth consent link (`/consent?request=…`) must survive the
 * login page, including a full-page provider round trip, so it is kept for the tab only.
 *
 * The path is kept with the id of the person who was signed in when it was remembered (null when nobody was,
 * as for an invitation link). Someone else signing in does not inherit it.
 */
import { lastUserId } from './userBoundary'

const KEY = 'penfii.afterLogin'

type Stored = { path: string; userId: string | null }

function isLocalPath(path: unknown): path is string {
  return typeof path === 'string' && path.startsWith('/') && !path.startsWith('//')
}

function read(): Stored | null {
  try {
    const raw = sessionStorage.getItem(KEY)
    if (!raw) return null
    if (raw.startsWith('/')) return isLocalPath(raw) ? { path: raw, userId: null } : null
    const parsed = JSON.parse(raw) as Partial<Stored>
    return isLocalPath(parsed.path) ? { path: parsed.path, userId: typeof parsed.userId === 'string' ? parsed.userId : null } : null
  } catch {
    return null
  }
}

export function rememberReturn(path: string): void {
  if (!path.startsWith('/') || path.startsWith('//') || path === '/' || path.startsWith('/login')) return
  try {
    sessionStorage.setItem(KEY, JSON.stringify({ path, userId: lastUserId() } satisfies Stored))
  } catch {
    // Storage can be blocked; the person lands on the home page instead.
  }
}

/** Forgets the remembered path. */
export function dropReturn(): void {
  try {
    sessionStorage.removeItem(KEY)
  } catch {
    // ignore
  }
}

/**
 * The remembered path once, or `fallback`. Only same-site relative paths are ever returned, and a path that
 * belongs to another signed-in person (`userId`) is dropped.
 */
export function takeReturn(fallback = '/', userId?: string): string {
  const stored = read()
  dropReturn()
  if (stored && (stored.userId === null || userId === undefined || stored.userId === userId)) return stored.path
  return fallback
}

export function peekReturn(): string | undefined {
  return read()?.path
}
