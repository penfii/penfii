import { useEffect } from 'react'
import { useQueryClient, type QueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { openLiveSocket, type LiveSocket } from '../../realtime/liveSocket'
import { dropChanges, handleProjectFrame, helloFor, newSyncState, type SyncState } from './projectSync'

/**
 * One socket per Project for the whole tab, shared by every screen of that Project (Board, Planning, Timeline).
 * It stays open for a moment after the last screen leaves, so moving between those screens does not reconnect, and
 * what the client holds (`lastSeq`) is kept for as long as the socket is.
 */
const LINGER_MS = 5000

type Entry = {
  refs: number
  state: SyncState
  live: LiveSocket | null
  close: number
}

const entries = new Map<string, Entry>()

function connect(queryClient: QueryClient, projectId: string, entry: Entry): void {
  // What the client holds (`state`) is the Project's; this only keeps a connection for it.
  entry.live = openLiveSocket({
    path: () => `/v1/ws/projects/${encodeURIComponent(projectId)}`,
    hello: () => helloFor(entry.state),
    probe: () => api.getProjectBoard(projectId),
    onFrame: (data) => {
      try {
        handleProjectFrame(queryClient, projectId, entry.state, JSON.parse(data) as unknown)
      } catch {
        // Ignore malformed frames and keep the last server snapshot.
      }
    },
  })
}

function shutdown(projectId: string, entry: Entry): void {
  window.clearTimeout(entry.close)
  dropChanges(entry.state)
  entry.live?.close()
  entries.delete(projectId)
}

/** Keeps the screens of a Project fresh from its change feed while the caller is mounted. Returns the release. */
export function acquireProjectLive(queryClient: QueryClient, projectId: string): () => void {
  let entry = entries.get(projectId)
  if (!entry) {
    entry = { refs: 0, state: newSyncState(), live: null, close: 0 }
    entries.set(projectId, entry)
    connect(queryClient, projectId, entry)
  }
  window.clearTimeout(entry.close)
  entry.refs += 1
  const held = entry
  return () => {
    held.refs -= 1
    if (held.refs <= 0) held.close = window.setTimeout(() => shutdown(projectId, held), LINGER_MS)
  }
}

/** Keeps a Project's Board, Planning, Timeline and open Tickets fresh from the server's change feed. */
export function useProjectLive(projectId: string): void {
  const queryClient = useQueryClient()
  useEffect(() => (projectId ? acquireProjectLive(queryClient, projectId) : undefined), [projectId, queryClient])
}

/** Closes every Project socket now (the session ended). */
export function closeAllProjectLive(): void {
  for (const [projectId, entry] of [...entries]) shutdown(projectId, entry)
}
