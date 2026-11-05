import { useQuery } from '@tanstack/react-query'
import { queries } from '../../queries'

/** How many unread Inbox items the person has, counting only what they can still open. */
export function useUnread(workspaceId: string) {
  const unread = useQuery({
    ...queries.inboxUnread(workspaceId),
    enabled: workspaceId.length > 0,
    refetchInterval: 60_000,
    refetchOnWindowFocus: true,
  })
  return { count: unread.data?.unreadCount ?? 0, capped: unread.data?.unreadCapped ?? false }
}
