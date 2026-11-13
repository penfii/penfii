import type { RichTextDoc } from '@penfii/contracts'
import { getAuthEpoch } from '@penfii/api-client'
import { api } from '../../api'
import { isBlankDoc } from '../../richtext'
import { errorCode, errorDetails } from '../work/ticketModel'
import { newInputSession } from './draftSession'

const SAVE_DELAY_MS = 800

export type DraftKind = 'CHANNEL_COMPOSE' | 'THREAD_COMPOSE' | 'DM_COMPOSE'

export type DraftController = {
  /** The compose box now holds `doc`. */
  update: (doc: RichTextDoc) => void
  /** The box goes away (another conversation, the screen left): what is waiting to be saved is saved now. */
  dispose: () => void
}

/**
 * Keeps what is typed in one compose box as the person's Draft on the server, so it survives a reload, follows them to
 * another device and shows up in Drafts.
 *
 * A Draft belongs to an input session: the time from the box being empty to it being typed in and then sent or emptied.
 * Every save names its session (`generation`), and sending or emptying the box ends it (`closeDraft`, which needs no
 * answer to an earlier save). The server refuses a save of an ended or older session, so what is still on its way when the
 * message is sent, or what another tab or a reloaded page sends late, cannot bring the Draft back. This controller also
 * sends one request at a time for its box, so its own saves arrive in the order they were made.
 *
 *  - it fills an empty box from the stored Draft and goes on with that Draft's session;
 *  - it saves a moment after typing stops, and at once when the box goes away with something unsaved;
 *  - it ends the session when the box is emptied (which is also what sending does);
 *  - it takes a newer stored version from another tab and goes on (the person is typing here now);
 *  - it never saves for a session it ended itself, whatever answers arrive late.
 */
export function createDraftController(options: {
  kind: DraftKind
  contextId: string
  /** The stored Draft is loaded into an empty box. */
  fill: (body: RichTextDoc) => void
}): DraftController {
  const { kind, contextId } = options
  const epoch = getAuthEpoch()
  let disposed = false
  /** The current input session; null until something is typed, and again after it ended. */
  let session: string | null = null
  let version: number | null = null
  /** The text of the last save that worked (or the loaded Draft), to save only what changed. */
  let last = ''
  /** A save of this session was asked for: a late one is possible, so ending the session must be told to the server. */
  let started = false
  let latest: RichTextDoc | null = null
  let timer = 0
  let queue: Promise<unknown> = Promise.resolve()

  const enqueue = (task: () => Promise<void>) => {
    queue = queue.then(task, task).catch(() => undefined)
  }

  function save(forSession: string): Promise<void> {
    return (async () => {
      // The box moved on (the session ended, or another began) before this turn: nothing to do for this one.
      if (session !== forSession || latest === null || isBlankDoc(latest)) return
      const body = latest
      const payload = { schemaVersion: 1 as const, body }
      const send = (expectedVersion: number | undefined) => api.saveDraft({ contextKind: kind, contextId, generation: forSession, expectedVersion, payload })
      try {
        let saved
        try {
          saved = await send(version ?? undefined)
        } catch (cause) {
          if (errorCode(cause) !== 'draft_conflict') throw cause
          // Another tab saved under the same session: take its version and go on.
          saved = await send(Number(errorDetails(cause).currentVersion ?? 0) || undefined)
        }
        if (session === forSession) {
          version = saved.version
          last = JSON.stringify(body)
        }
      } catch (cause) {
        if (errorCode(cause) === 'draft_closed' && session === forSession && !disposed) {
          // Ended elsewhere (another tab sent it, it was discarded in Drafts) or older than a session started elsewhere,
          // while the person is still typing here: this text is theirs now, so it starts a session of its own.
          session = newInputSession()
          version = null
          last = ''
          started = true
          const fresh = session
          enqueue(() => save(fresh))
        }
        // Anything else: the text stays in the box; the next change tries again.
      }
    })()
  }

  function schedule() {
    window.clearTimeout(timer)
    timer = window.setTimeout(() => {
      timer = 0
      if (session === null || disposed) return
      const forSession = session
      started = true
      enqueue(() => save(forSession))
    }, SAVE_DELAY_MS)
  }

  function end() {
    window.clearTimeout(timer)
    timer = 0
    const ended = session
    const askedFor = started
    session = null
    version = null
    last = ''
    started = false
    // Nothing was ever asked of the server for this session: nothing can arrive late, so there is nothing to end there.
    if (ended === null || !askedFor) return
    // At once, not behind a save that is still on its way: the server makes the order of the two irrelevant.
    void api.closeDraft(kind, contextId, ended).catch(() => {
      // Best effort: a Draft that could not be ended is replaced by the next session's save.
    })
  }

  // Load the stored Draft into an empty box.
  void (async () => {
    try {
      const stored = await api.getDraft(kind, contextId)
      if (!stored || disposed || session !== null) return
      session = stored.generation
      version = stored.version
      last = JSON.stringify(stored.payload.body)
      started = true
      options.fill(stored.payload.body)
    } catch {
      // No stored draft (or none reachable): the box simply starts empty.
    }
  })()

  return {
    update(doc) {
      if (disposed) return
      latest = doc
      if (isBlankDoc(doc)) {
        // A loaded Draft is not blank, so this is a box that was emptied (sent, or cleared by hand).
        if (session !== null) end()
        return
      }
      if (session === null) session = newInputSession()
      if (JSON.stringify(doc) === last) {
        window.clearTimeout(timer)
        timer = 0
        return
      }
      schedule()
    },
    dispose() {
      if (disposed) return
      // What is waiting for the delay is saved now, unless the session of the person it belongs to is over.
      const pending = timer !== 0 && session !== null && latest !== null && !isBlankDoc(latest)
      window.clearTimeout(timer)
      timer = 0
      if (pending && epoch === getAuthEpoch()) {
        const forSession = session as string
        started = true
        enqueue(() => save(forSession))
      }
      disposed = true
    },
  }
}
