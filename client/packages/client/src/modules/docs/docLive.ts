import { useEffect } from 'react'
import { useQueryClient, type QueryClient } from '@tanstack/react-query'
import { isNotFound } from '@penfii/api-client'
import type { DocPage } from '@penfii/contracts'
import { api } from '../../api'
import { queryKeys } from '../../queryKeys'
import { openLiveSocket } from '../../realtime/liveSocket'

/** Notices of one burst (a Save sends a few) are read once: the page is read this long after the last of them. */
const SETTLE_MS = 150

type Frame = { type?: string; docId?: string }

/**
 * Keeps an open Doc page current. The server tells the socket of a page that it changed (a Save, the title, the
 * width, who may read it, a Version, the deletion) and nothing else: no content travels on it. The page is then read
 * again through the ordinary API, so what a person sees is always what that API says for them now, and a person who
 * lost the page learns it as the page's own 404.
 *
 *  - Notices that arrive together, repeat, or come in another order are read as one read, and a notice that arrives
 *    while a read is under way is followed by exactly one more read.
 *  - A connection that opens again reads once, whatever it missed while it was closed.
 *  - `doc.gone` (deleted, or this person lost it) closes the socket at once, so it does not try to come back to a page that
 *    answers 404 before the read has said so; if the read finds the page readable after all, the socket is opened again.
 *  - What a read cannot change is not touched: the saved document is replaced in the cache and nothing else. A draft
 *    being edited lives apart from it (`DocPageView` keeps its own draft and base revision).
 *  - Reading is the page's own doing, not a person's: the API client marks it as background once the person has
 *    been idle, so it never keeps a long session alive.
 */
export function startDocLive(queryClient: QueryClient, docId: string, workspaceId: string): () => void {
  const key = queryKeys.doc(docId)
  let timer = 0
  let running = false
  let again = false
  let stopped = false
  let closedByGone = false

  const read = async () => {
    if (stopped) return
    if (running) {
      again = true
      return
    }
    running = true
    const before = queryClient.getQueryData<DocPage>(key)
    try {
      await queryClient.refetchQueries({ queryKey: key, exact: true }, { cancelRefetch: false })
      const after = queryClient.getQueryData<DocPage>(key)
      const state = queryClient.getQueryState(key)
      if (state?.status === 'error' && isNotFound(state.error)) {
        // Deleted, or no longer readable by this person: the socket has nothing more to say.
        live.close()
        stopped = true
        return
      }
      if (closedByGone && state?.status === 'success') {
        // The page is readable after all (a notice raced with a change that gave it back): follow it again.
        closedByGone = false
        live = connect()
      }
      // The tree shows the title and who may read a page; History lists the Versions; references follow the content.
      if (before && after && (before.revisionNo !== after.revisionNo || before.title !== after.title || before.visibility !== after.visibility || before.latestVersionNo !== after.latestVersionNo)) {
        void queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) })
        void queryClient.invalidateQueries({ queryKey: queryKeys.docVersions(docId) })
        void queryClient.invalidateQueries({ queryKey: queryKeys.all.docReferences })
      }
    } finally {
      running = false
      if (again && !stopped) {
        again = false
        schedule()
      }
    }
  }
  const schedule = () => {
    if (stopped) return
    window.clearTimeout(timer)
    timer = window.setTimeout(() => void read(), SETTLE_MS)
  }

  const connect = () => openLiveSocket({
    path: () => `/v1/ws/docs/${docId}`,
    // The probe is the page itself: a person who may not open the socket (the page was deleted, access was taken
    // away) is answered 404 here and the socket stops trying.
    probe: () => api.getDoc(docId),
    onFrame: (data) => {
      let frame: Frame
      try {
        frame = JSON.parse(data) as Frame
      } catch {
        return
      }
      if (frame.docId !== undefined && frame.docId !== docId) return
      if (frame.type === 'doc.gone' && !closedByGone) {
        closedByGone = true
        live.close()
      }
      // `doc.ready` is the first frame of every connection: what changed before it was subscribed (or while the
      // connection was down) is read now.
      if (frame.type === 'doc.ready' || frame.type === 'doc.changed' || frame.type === 'doc.gone') schedule()
    },
    onStop: () => schedule(),
  })
  let live = connect()
  return () => {
    stopped = true
    window.clearTimeout(timer)
    live.close()
  }
}

/** The live updates of an open page. */
export function useDocLive(docId: string, workspaceId: string): void {
  const queryClient = useQueryClient()
  useEffect(() => startDocLive(queryClient, docId, workspaceId), [queryClient, docId, workspaceId])
}
