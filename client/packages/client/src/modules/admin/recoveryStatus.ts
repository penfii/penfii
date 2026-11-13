import type { MessageKey } from '../../i18n/messages'

/** What a recovery section says about the last attempt; it outlives the row that caused it. */
export type RecoveryStatus = { tone: 'ok' | 'error'; key: MessageKey }
