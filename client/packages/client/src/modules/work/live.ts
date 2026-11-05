import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { refreshTicket } from './refresh'
import { queryKeys } from '../../queryKeys'

/** Keeps a Project's Board and open Tickets fresh from the server's change feed. */
export function useProjectLive(projectId: string) {
  const queryClient = useQueryClient()
  useEffect(() => {
    if (typeof WebSocket !== 'function') return
    let socket: WebSocket | null = null
    let closed = false
    let timer = 0
    const connect = () => {
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      socket = new WebSocket(`${protocol}//${window.location.host}/v1/ws/projects/${encodeURIComponent(projectId)}`)
      socket.onopen = () => socket?.send(JSON.stringify({ v: 1, type: 'hello', afterChangeSeq: 0 }))
      socket.onmessage = (event) => {
        if (typeof event.data !== 'string') return
        try {
          const frame = JSON.parse(event.data) as { type?: string; projectId?: string; ticketId?: string }
          if (frame.projectId !== projectId) return
          if (frame.type === 'project.ticket.change' && frame.ticketId) {
            void refreshTicket(queryClient, frame.ticketId, projectId)
          } else if (frame.type === 'project.ready' || frame.type === 'project.resync_required') {
            void queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) })
          }
        } catch {
          // Ignore malformed frames and keep the last server snapshot.
        }
      }
      socket.onclose = () => {
        if (!closed) timer = window.setTimeout(connect, 1500)
      }
    }
    const start = window.setTimeout(connect, 0)
    return () => {
      closed = true
      window.clearTimeout(start)
      window.clearTimeout(timer)
      socket?.close()
    }
  }, [projectId, queryClient])
}
