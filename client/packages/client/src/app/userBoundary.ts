import type { QueryClient } from '@tanstack/react-query'
import { endSession } from '@penfii/api-client'
import { LOCALE_STORAGE_KEY } from '../i18n/messages'
import { THEME_KEY } from '../design/theme'
import { closeAllProjectLive } from '../modules/work/live'
import { NAV_WIDTH_KEY, THREAD_WIDTH_KEY, TICKET_WIDTH_KEY } from '../shell/paneWidth'

/**
 * Whose state this tab holds. Everything one person reads or writes (query cache, requests in flight, drafts,
 * remembered places) belongs to them: it is cleared when they sign out, when the session ends, and when someone
 * else signs in. Query keys do not carry the person; the clearing at these moments is what separates people.
 */
const LAST_USER_KEY = 'penfii.lastUserId'
const RETURN_KEY = 'penfii.afterLogin'

/** Settings of the device, not of a person: kept across people. Every other `penfii.*` key is a person's. */
const DEVICE_KEYS = new Set([LOCALE_STORAGE_KEY, THEME_KEY, NAV_WIDTH_KEY, THREAD_WIDTH_KEY, TICKET_WIDTH_KEY, LAST_USER_KEY, RETURN_KEY])

export function lastUserId(): string | null {
  try {
    return localStorage.getItem(LAST_USER_KEY)
  } catch {
    return null
  }
}

function purge(store: Storage): void {
  const keys: string[] = []
  for (let index = 0; index < store.length; index += 1) {
    const key = store.key(index)
    if (key?.startsWith('penfii.') && !DEVICE_KEYS.has(key)) keys.push(key)
  }
  for (const key of keys) store.removeItem(key)
}

/** Removes what the last person saved in this browser (unsent documents, remembered views, seen markers, …). */
export function purgeUserStorage(): void {
  for (const open of [() => localStorage, () => sessionStorage]) {
    try {
      purge(open())
    } catch {
      // A blocked storage holds nothing to purge.
    }
  }
}

/**
 * The session is over (signed out, expired). Requests of it are aborted and their answers refused, the cache goes,
 * and with `forget` the person's saved state too. Cheap to call twice. A person whose session merely expired keeps
 * their saved state, so signing in again picks up where they were; it is purged if someone else signs in.
 */
export function endUserSession(queryClient: QueryClient, options: { forget: boolean }): void {
  endSession()
  closeAllProjectLive()
  void queryClient.cancelQueries()
  queryClient.clear()
  if (options.forget) {
    purgeUserStorage()
    try {
      sessionStorage.removeItem(RETURN_KEY)
      localStorage.removeItem(LAST_USER_KEY)
    } catch {
      // ignore
    }
  }
}

/** A person is signed in. When they are not who was signed in before, the earlier person's saved state is dropped. */
export function adoptUser(userId: string): boolean {
  const before = lastUserId()
  const changed = before !== null && before !== userId
  if (changed) {
    purgeUserStorage()
    try {
      sessionStorage.removeItem(RETURN_KEY)
    } catch {
      // ignore
    }
  }
  try {
    localStorage.setItem(LAST_USER_KEY, userId)
  } catch {
    // ignore
  }
  return changed
}
