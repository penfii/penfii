import type { MessageKey } from '../../i18n/messages'
import { errorCode, errorDetails } from './ticketModel'

/** The message key that explains why a State change or a move was refused. */
export function moveErrorKey(cause: unknown): MessageKey {
  const code = errorCode(cause)
  if (code === 'version_conflict') return 'work.move.conflict'
  if (code === 'unfinished_subtasks') return 'work.move.unfinished'
  if (code === 'dependency_blocked') {
    // A canceled predecessor can never finish; say so, because the way out is different.
    return Number(errorDetails(cause).canceledBlockers ?? 0) > 0 ? 'work.move.blockedCanceled' : 'work.move.blocked'
  }
  if (code === 'stale_anchor') return 'work.move.staleAnchor'
  if (code === 'forbidden') return 'work.move.notAllowed'
  return 'work.move.error'
}
