import { useEffect, useRef } from 'react'
import type { RichTextDoc } from '@penfii/contracts'
import { api } from '../../api'
import { isBlankDoc } from '../../richtext'
import { errorCode, errorDetails } from '../work/ticketModel'

const SAVE_DELAY_MS = 800

/**
 * Keeps what is typed in a compose box as the person's Draft on the server, so it survives a
 * reload, follows them to another device and shows up in Drafts. It loads the stored text only into
 * an empty box, saves a moment after typing stops, and removes the Draft when the box is emptied
 * (which is also what sending does). A save that finds a newer version from another tab takes that
 * version and goes on: the person is typing here now.
 */
export function useServerDraft(kind: 'CHANNEL_COMPOSE' | 'THREAD_COMPOSE' | 'DM_COMPOSE', contextId: string | null, doc: RichTextDoc, setDoc: (update: (current: RichTextDoc) => RichTextDoc) => void) {
  const state = useRef<{ id: string | null; version: number | null; ready: boolean; last: string; context: string | null }>({ id: null, version: null, ready: false, last: '', context: null })
  const docRef = useRef(doc)
  docRef.current = doc

  useEffect(() => {
    const current = state.current
    current.id = null
    current.version = null
    current.ready = false
    current.last = ''
    current.context = contextId
    if (!contextId) return
    let cancelled = false
    void (async () => {
      try {
        const draft = await api.getDraft(kind, contextId)
        if (cancelled) return
        if (draft) {
          current.id = draft.id
          current.version = draft.version
          current.last = JSON.stringify(draft.payload.body)
          setDoc((existing) => (isBlankDoc(existing) ? draft.payload.body : existing))
        }
      } catch {
        // No stored draft (or none reachable): the box simply starts empty.
      }
      if (!cancelled) current.ready = true
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, contextId])

  useEffect(() => {
    const current = state.current
    if (!contextId || !current.ready || current.context !== contextId) return
    const serialized = JSON.stringify(doc)
    if (isBlankDoc(doc)) {
      if (current.id === null) return
      const id = current.id
      current.id = null
      current.version = null
      current.last = ''
      void (async () => {
        try {
          await api.deleteDraft(id)
        } catch {
          // A draft that could not be removed is replaced by the next save.
        }
      })()
      return
    }
    if (serialized === current.last) return
    const timer = window.setTimeout(() => {
      void (async () => {
        const body = docRef.current
        const payload = { schemaVersion: 1 as const, body }
        try {
          let saved
          try {
            saved = await api.saveDraft({ contextKind: kind, contextId, expectedVersion: current.version ?? undefined, payload })
          } catch (cause) {
            if (errorCode(cause) !== 'draft_conflict') throw cause
            saved = await api.saveDraft({ contextKind: kind, contextId, expectedVersion: Number(errorDetails(cause).currentVersion ?? 0) || undefined, payload })
          }
          if (current.context === contextId) {
            current.id = saved.id
            current.version = saved.version
            current.last = JSON.stringify(body)
          }
        } catch {
          // The text stays in the box; the next change tries again.
        }
      })()
    }, SAVE_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [doc, contextId, kind])
}
