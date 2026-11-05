import type { Dm } from '@penfii/contracts'
import type { NavigateFunction } from 'react-router'

export const PENDING_DM_KEY = 'penfii.pendingDm'

export type PendingPeer = { id: string; displayName: string }

export function rememberPeers(peers: PendingPeer[]) {
  try {
    sessionStorage.setItem(PENDING_DM_KEY, JSON.stringify(peers))
  } catch {
    // The address still carries the user ids.
  }
}

export function readPeers(ids: string[]): PendingPeer[] {
  try {
    const parsed = JSON.parse(sessionStorage.getItem(PENDING_DM_KEY) ?? '[]') as unknown
    if (!Array.isArray(parsed)) return []
    return parsed.flatMap((item) => {
      if (!item || typeof item !== 'object') return []
      const peer = item as PendingPeer
      if (typeof peer.id !== 'string' || typeof peer.displayName !== 'string' || !ids.includes(peer.id)) return []
      return [peer]
    })
  } catch {
    return []
  }
}

export function startedConversation(dms: Dm[], memberIds: string[]): Dm | undefined {
  const want = [...memberIds].sort()
  return dms.find((dm) => {
    if ((dm.lastMessageSeq ?? 0) <= 0) return false
    const ids = [...dm.memberIds].sort()
    return ids.length === want.length && ids.every((id, index) => id === want[index])
  })
}

export function openPendingDm(
  navigate: NavigateFunction,
  workspaceId: string,
  peers: PendingPeer[],
) {
  rememberPeers(peers)
  void navigate(`/w/${workspaceId}/dm/new?users=${peers.map((peer) => peer.id).join(',')}`, { state: { peers } })
}
