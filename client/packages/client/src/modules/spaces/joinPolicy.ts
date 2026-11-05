/**
 * Whether the composer of a channel is a Join action. Reading a public channel needs no membership; writing
 * does, except in a public channel of a Project or Team, which its own people write without joining (the
 * server's `ConvAccess::can_write`). A channel the person can list but not write in only occurs in the
 * first case, so the list's `joined` flag is enough.
 */
export function shouldOfferJoin(channel: { visibility: string; joined?: boolean; projectId?: string | null; teamId?: string | null }): boolean {
  if (channel.visibility !== 'PUBLIC' || channel.joined !== false) return false
  return !channel.projectId && !channel.teamId
}
