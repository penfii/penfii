import { isApiError } from '@penfii/api-client'

/**
 * The connection policy of every live socket (user, Project, conversation): when to connect, how long to wait after a
 * failure, when to stop. What the frames mean, and what to send first, stays with each socket: the caller gives
 * `onFrame` and `hello` and this reconnects around them. In particular what a reconnect has to catch up on (the Project's
 * `lastSeq`, the conversation's cursor) is the caller's, and `hello` is asked for again on every connection.
 *
 *  - Failures wait 1 s, 2 s, 4 s … up to 30 s, each with full jitter (a random delay up to that), so clients of a
 *    restarted server do not return together. The count starts again once the server has answered.
 *  - A person closing it (`close()`, the screen left, the session ended) is not a failure and is never retried.
 *  - A socket the server refuses cannot be told from one that is down: the browser reports no status. After a few
 *    connections that never got an answer, `probe` (one ordinary request) asks. When it says the person may not
 *    open this (401, 403, 404) the socket stops and `onStop` is told; any other answer is a server that is down, and
 *    the attempts go on.
 *  - Offline, no attempt is made; coming back online, or the tab becoming visible, tries at once.
 */
const BASE_MS = 1000
const CAP_MS = 30000
/** A connection that stayed open this long counts as having worked, even if the server sent nothing. */
const STABLE_MS = 10000
/** Connections in a row that got no answer before `probe` is asked. */
const UNANSWERED_BEFORE_PROBE = 3

export type LiveSocketOptions = {
  /** The path of the socket, e.g. `/v1/ws/user`; called for every connection. */
  path: () => string
  /** The first message of a connection (null: none). Called again for every connection. */
  hello?: () => unknown
  onFrame: (data: string) => void
  /** The socket opened; `reconnected` is false for the first connection. */
  onOpen?: (reconnected: boolean) => void
  /** One ordinary request that fails (401, 403, 404) when this person may not use the socket. */
  probe?: () => Promise<unknown>
  /** The probe said the person may not open it: no more attempts. */
  onStop?: (status: number) => void
  /** For tests. */
  random?: () => number
}

export type LiveSocket = { close: () => void }

export function backoffMs(failures: number, random: () => number): number {
  return Math.floor(random() * Math.min(CAP_MS, BASE_MS * 2 ** Math.max(0, failures - 1)))
}

export function openLiveSocket(options: LiveSocketOptions): LiveSocket {
  if (typeof WebSocket !== 'function') return { close: () => undefined }
  const random = options.random ?? Math.random
  let socket: WebSocket | null = null
  let stopped = false
  let timer = 0
  let stableTimer = 0
  let failures = 0
  let unanswered = 0
  let everOpened = false
  let probing = false

  const clearTimers = () => {
    window.clearTimeout(timer)
    window.clearTimeout(stableTimer)
    timer = 0
    stableTimer = 0
  }

  const schedule = () => {
    if (stopped || timer) return
    if (typeof navigator !== 'undefined' && navigator.onLine === false) return
    failures += 1
    timer = window.setTimeout(() => {
      timer = 0
      connect()
    }, backoffMs(failures, random))
  }

  const healthy = () => {
    failures = 0
    unanswered = 0
  }

  async function checkAllowed(): Promise<boolean> {
    if (!options.probe || probing) return true
    probing = true
    try {
      await options.probe()
      return true
    } catch (caught) {
      if (isApiError(caught) && (caught.status === 401 || caught.status === 403 || caught.status === 404)) {
        if (!stopped) {
          stopped = true
          clearTimers()
          options.onStop?.(caught.status)
        }
        return false
      }
      return true
    } finally {
      probing = false
    }
  }

  function connect() {
    if (stopped) return
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const current = new WebSocket(`${protocol}//${window.location.host}${options.path()}`)
    socket = current
    let answered = false
    current.onopen = () => {
      options.onOpen?.(everOpened)
      everOpened = true
      const hello = options.hello?.()
      if (hello !== undefined && hello !== null) current.send(JSON.stringify(hello))
      stableTimer = window.setTimeout(() => {
        answered = true
        healthy()
      }, STABLE_MS)
    }
    current.onmessage = (event) => {
      if (typeof event.data !== 'string') return
      if (!answered) {
        answered = true
        healthy()
      }
      options.onFrame(event.data)
    }
    current.onclose = () => {
      if (socket === current) socket = null
      window.clearTimeout(stableTimer)
      if (stopped) return
      if (!answered) unanswered += 1
      if (unanswered >= UNANSWERED_BEFORE_PROBE) {
        void checkAllowed().then((allowed) => {
          if (allowed) schedule()
        })
        return
      }
      schedule()
    }
  }

  const retryNow = () => {
    if (stopped || socket || probing) return
    window.clearTimeout(timer)
    timer = 0
    connect()
  }
  const onVisible = () => {
    if (document.visibilityState === 'visible') retryNow()
  }
  window.addEventListener('online', retryNow)
  document.addEventListener('visibilitychange', onVisible)
  // Not in the same turn: a screen that mounts and unmounts at once (StrictMode) opens nothing.
  timer = window.setTimeout(() => {
    timer = 0
    connect()
  }, 0)

  return {
    close() {
      stopped = true
      clearTimers()
      window.removeEventListener('online', retryNow)
      document.removeEventListener('visibilitychange', onVisible)
      const open = socket
      socket = null
      if (open) {
        open.onopen = null
        open.onmessage = null
        open.onclose = null
        open.close()
      }
    },
  }
}
