import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { queryKeys } from '../../queryKeys'
import { openLiveSocket } from '../../realtime/liveSocket'

/**
 * One socket for the signed-in person. It only says "your Inbox changed" or "your channels changed"; the page then asks the
 * API, so nothing private travels on the socket. A missed notice heals on focus or the next poll. Connecting, waiting
 * between attempts and stopping are `openLiveSocket`'s.
 */
export function useUserSocket() {
  const queryClient = useQueryClient()
  useEffect(() => {
    let connectedBefore = false
    const live = openLiveSocket({
      path: () => '/v1/ws/user',
      probe: () => api.me(),
      onFrame: (data) => {
        try {
          const frame = JSON.parse(data) as { type?: string }
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
      },
    })
    return () => live.close()
  }, [queryClient])
}
