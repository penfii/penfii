import { createContext, useCallback, useContext, useSyncExternalStore } from 'react'
import type { ReferenceKind, ResolvedReference, Space } from '@penfii/contracts'
import type { MentionName } from './text'

/**
 * What the rich text renderer needs from the app and cannot know itself: the names behind ids and
 * the titles behind internal links. The shells provide it; without it (a test, a standalone view)
 * mentions show what the document stores and links stay plain links.
 */
export type RichTextEnv = {
  workspaceId: string
  /** A Ticket, Doc or person title, resolved with the viewer's permissions in batches. */
  resolver: {
    subscribe: (listener: () => void) => () => void
    getVersion: () => number
    want: (kind: ReferenceKind, id: string) => ResolvedReference | undefined
  }
  space: Space | undefined
  /** Moves within the app (a plain click on an in-app link). */
  navigate: (to: string) => void
}

export const RichTextEnvContext = createContext<RichTextEnv | null>(null)

export function useRichTextEnv(): RichTextEnv | null {
  return useContext(RichTextEnvContext)
}

const NO_VERSION = () => 0
const NO_SUBSCRIBE = () => () => undefined

/** Re-renders when a resolver answer arrives. */
export function useResolverVersion(env: RichTextEnv | null): void {
  useSyncExternalStore(env ? env.resolver.subscribe : NO_SUBSCRIBE, env ? env.resolver.getVersion : NO_VERSION)
}

/**
 * The name behind a person's id: a name the caller already knows (`names`), else the one the server
 * gives for people the viewer may see. Undefined while it is being asked for or when unavailable.
 */
export function useMentionName(userId: string, names?: Readonly<Record<string, string>>): { name: string | undefined; pending: boolean } {
  const env = useRichTextEnv()
  useResolverVersion(env)
  const known = names?.[userId]
  if (known) return { name: known, pending: false }
  if (!env) return { name: undefined, pending: false }
  const answer = env.resolver.want('USER', userId)
  if (!answer) return { name: undefined, pending: true }
  return { name: answer.available ? answer.title : undefined, pending: false }
}

/** `nameOf` for `plainText` (copy, quotes, Ticket prefill): resolves what it is asked for. */
export function useMentionNameOf(names?: Readonly<Record<string, string>>): MentionName {
  const env = useRichTextEnv()
  useResolverVersion(env)
  const version = env?.resolver.getVersion() ?? 0
  return useCallback(
    (userId: string) => {
      const known = names?.[userId]
      if (known) return known
      const answer = env?.resolver.want('USER', userId)
      return answer?.available ? answer.title : undefined
    },
    // A new version means answers arrived: a new callback makes callers read again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [env, names, version],
  )
}
