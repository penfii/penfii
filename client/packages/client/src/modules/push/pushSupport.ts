/**
 * What this browser can do about Web Push, decided from facts about the browser. No request is made
 * here and nothing is stored.
 */
export type PushEnvironment = {
  hasServiceWorker: boolean
  hasPushManager: boolean
  hasNotification: boolean
  userAgent: string
  platform: string
  maxTouchPoints: number
  /** The page runs as an installed app (Home Screen), not in a browser tab. */
  standalone: boolean
}

export type PushSupport =
  /** This browser cannot do Web Push at all. */
  | 'unsupported'
  /** iPhone/iPad in a Safari tab: Web Push works only after "Add to Home Screen". */
  | 'ios-needs-install'
  | 'supported'

export function isIos(env: Pick<PushEnvironment, 'userAgent' | 'platform' | 'maxTouchPoints'>): boolean {
  if (/iPad|iPhone|iPod/.test(env.userAgent)) return true
  // iPadOS 13+ reports itself as a Mac with a touch screen.
  return env.platform === 'MacIntel' && env.maxTouchPoints > 1
}

export function pushSupport(env: PushEnvironment): PushSupport {
  const complete = env.hasServiceWorker && env.hasPushManager && env.hasNotification
  if (isIos(env) && !env.standalone) return 'ios-needs-install'
  return complete ? 'supported' : 'unsupported'
}

export function currentEnvironment(): PushEnvironment {
  const nav = typeof navigator === 'undefined' ? undefined : navigator
  const standalone =
    (nav as (Navigator & { standalone?: boolean }) | undefined)?.standalone === true ||
    (typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(display-mode: standalone)').matches)
  return {
    hasServiceWorker: Boolean(nav && 'serviceWorker' in nav),
    hasPushManager: typeof window !== 'undefined' && 'PushManager' in window,
    hasNotification: typeof window !== 'undefined' && 'Notification' in window,
    userAgent: nav?.userAgent ?? '',
    platform: nav?.platform ?? '',
    maxTouchPoints: nav?.maxTouchPoints ?? 0,
    standalone,
  }
}

/** `applicationServerKey` wants bytes; the server gives base64url. */
export function urlBase64ToBytes(value: string): Uint8Array<ArrayBuffer> {
  const padded = value.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(value.length / 4) * 4, '=')
  const raw = atob(padded)
  const bytes = new Uint8Array(new ArrayBuffer(raw.length))
  for (let i = 0; i < raw.length; i += 1) bytes[i] = raw.charCodeAt(i)
  return bytes
}

/** A short name for this device in the list ("Chrome on Windows"). */
export function deviceLabel(userAgent: string): string {
  const browser = /Edg\//.test(userAgent) ? 'Edge' : /Firefox\//.test(userAgent) ? 'Firefox' : /Chrome\//.test(userAgent) ? 'Chrome' : /Safari\//.test(userAgent) ? 'Safari' : 'Browser'
  const system = /iPhone/.test(userAgent) ? 'iPhone' : /iPad/.test(userAgent) ? 'iPad' : /Android/.test(userAgent) ? 'Android' : /Windows/.test(userAgent) ? 'Windows' : /Mac OS X|Macintosh/.test(userAgent) ? 'Mac' : /Linux/.test(userAgent) ? 'Linux' : ''
  return system ? `${browser} · ${system}` : browser
}
