import { isApiError } from '@penfii/api-client'

export { LABEL_MAX as CHANNEL_NAME_MAX, labelProblem as channelNameProblem } from '../ui/labelRule'

/** The server's rule for a channel description: optional; trimmed, at most 2000 characters, no control characters (a pasted tab is one). */
export const CHANNEL_DESCRIPTION_MAX = 2000

export function descriptionProblem(value: string): boolean {
  const trimmed = value.trim()
  return trimmed !== '' && ([...trimmed].length > CHANNEL_DESCRIPTION_MAX || /\p{Cc}/u.test(trimmed))
}

/**
 * What a failed `createChannel` tells the person. `offline` means the request never left the browser;
 * `uncertain` means it may have reached the server (network error, 5xx, timeout), so a channel may exist.
 */
export type CreateFailure = 'name' | 'offline' | 'forbidden' | 'teamArchived' | 'notFound' | 'uncertain' | 'rejected'

export function classifyCreateFailure(caught: unknown): CreateFailure {
  if (!isApiError(caught)) return 'uncertain'
  if (caught.code === 'offline' && caught.status === 0) return 'offline'
  if (caught.code === 'team_archived') return 'teamArchived'
  // The form checks everything else the server validates (description, visibility, team) before sending, so a refusal is the name's.
  if (caught.status === 400 && caught.code === 'validation') return 'name'
  if (caught.status === 403) return 'forbidden'
  if (caught.status === 404) return 'notFound'
  if (caught.status >= 400 && caught.status < 500) return 'rejected'
  return 'uncertain'
}
