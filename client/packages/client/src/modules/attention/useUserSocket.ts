import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '../../queryKeys'

/**
 * One socket for the signed-in person. It only says "your Inbox changed" or "your channels changed"; the page then asks the
 * API, so nothing private travels on the socket. A missed notice heals on focus or the next poll.
 */
export function useUserSocket() {
  const queryClient = useQueryClient()
  useEffect(() => {
    if (typeof WebSocket !== 'function') return
    let socket: WebSocket | null = null
    let closed = false
    let timer = 0
    let connectedBefore = false
    const connect = () => {
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      socket = new WebSocket(`${protocol}//${window.location.host}/v1/ws/user`)
      socket.onmessage = (event) => {
        if (typeof event.data !== 'string') return
        try {
          const frame = JSON.parse(event.data) as { type?: string }
          if (frame.type === 'inbox.changed') {
            void queryClient.invalidateQueries({ queryKey: queryKeys.all.inbox })
            void queryClient.invalidateQueries({ queryKey: queryKeys.all.inboxUnread })
          }
          if (frame.type === 'user.ready') {
            // A socket that opens again (after a drop, or after access was re-checked) may have missed
            // notices while it was closed, so what they announce is read once more.
            if (connectedBefore) {
              void queryClient.invalidateQueries({ queryKey: queryKeys.all.space })
              void queryClient.invalidateQueries({ queryKey: queryKeys.all.inbox })
              void queryClient.invalidateQueries({ queryKey: queryKeys.all.inboxUnread })
            }
            connectedBefore = true
          }
          if (frame.type === 'space.changed') {
            // Put into a private channel or left one: every Workspace's channel list is read again.
            void queryClient.invalidateQueries({ queryKey: queryKeys.all.space })
          }
        } catch {
          // A malformed frame is ignored; the next poll catches up.
        }
      }
      socket.onclose = () => {
        if (!closed) timer = window.setTimeout(connect, 3000)
      }
    }
    const start = window.setTimeout(connect, 0)
    return () => {
      closed = true
      window.clearTimeout(start)
      window.clearTimeout(timer)
      socket?.close()
    }
  }, [queryClient])
}
