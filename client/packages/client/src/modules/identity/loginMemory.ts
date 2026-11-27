/**
 * What the sign-in page remembers in this browser (and so, per instance: storage is per origin). Only things that
 * make signing in easier and are not secrets: an e-mail address the person chose to keep, which method last worked,
 * and the "keep me signed in" choice. A password, a session token, an OAuth token or a code is never stored here.
 *
 * Every read and write is guarded: with storage blocked or full, nothing is remembered and sign-in works as before.
 * These keys are device settings (see `userBoundary`): signing out does not clear them, the person removes them.
 */
export const LOGIN_EMAIL_KEY = 'penfii.login.email'
export const LOGIN_METHOD_KEY = 'penfii.login.method'
export const LOGIN_KEEP_KEY = 'penfii.login.keepSignedIn'
/** A provider sign-in that was started and not yet confirmed (session storage: it belongs to this tab's round trip). */
export const LOGIN_PENDING_KEY = 'penfii.login.pending'

/** A provider sign-in that was started more than this long ago is not confirmed any more. */
const PENDING_MAX_MS = 15 * 60 * 1000

function read(store: () => Storage, key: string): string | null {
  try {
    return store().getItem(key)
  } catch {
    return null
  }
}

function write(store: () => Storage, key: string, value: string | null): void {
  try {
    if (value === null) store().removeItem(key)
    else store().setItem(key, value)
  } catch {
    // Storage is blocked or full: nothing is remembered.
  }
}

const local = () => localStorage
const session = () => sessionStorage

/** The kept e-mail address, or null. A value that does not look like an address is ignored. */
export function rememberedEmail(): string | null {
  const value = read(local, LOGIN_EMAIL_KEY)?.trim() ?? ''
  return value.length > 0 && value.length <= 320 && value.includes('@') ? value : null
}

export function rememberEmail(email: string): void {
  write(local, LOGIN_EMAIL_KEY, email.trim())
}

export function forgetEmail(): void {
  write(local, LOGIN_EMAIL_KEY, null)
}

/** `password`, or the key of the provider (`google`, `oidc:corp`) that last signed this browser in. */
export function lastMethod(): string | null {
  const value = read(local, LOGIN_METHOD_KEY)?.trim() ?? ''
  return value.length > 0 && value.length <= 100 ? value : null
}

export function setLastMethod(method: string): void {
  write(local, LOGIN_METHOD_KEY, method)
}

export function keepSignedInChoice(): boolean {
  return read(local, LOGIN_KEEP_KEY) === '1'
}

export function setKeepSignedInChoice(on: boolean): void {
  write(local, LOGIN_KEEP_KEY, on ? '1' : '0')
}

/** A person pressed a provider's sign-in button. This records nothing about success; see `confirmProviderLogin`. */
export function markProviderAttempt(provider: string): void {
  write(session, LOGIN_PENDING_KEY, JSON.stringify({ provider, at: Date.now() }))
}

export function clearProviderAttempt(): void {
  write(session, LOGIN_PENDING_KEY, null)
}

/** The pending provider sign-in, read and cleared; null when there is none, it is stale or unreadable. */
export function takeProviderAttempt(now = Date.now()): { provider: string; at: number } | null {
  const raw = read(session, LOGIN_PENDING_KEY)
  clearProviderAttempt()
  if (!raw) return null
  try {
    const value = JSON.parse(raw) as { provider?: unknown; at?: unknown }
    if (typeof value.provider !== 'string' || typeof value.at !== 'number') return null
    return now - value.at <= PENDING_MAX_MS && value.at <= now + 60_000 ? { provider: value.provider, at: value.at } : null
  } catch {
    return null
  }
}
