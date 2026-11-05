import { useSyncExternalStore } from 'react'
import type { ReferenceKind } from '@penfii/contracts'
import { useI18n } from '../../i18n/I18nProvider'
import { useDocContext } from './docContext'

const NO_VERSION = () => 0
const NO_SUBSCRIBE = () => () => undefined

/** Subscribes to the page's resolver and returns the answer for one node, asking for it if needed. */
export function useResolved(kind: ReferenceKind, id: string) {
  const { resolver } = useDocContext()
  useSyncExternalStore(resolver ? resolver.subscribe : NO_SUBSCRIBE, resolver ? resolver.getVersion : NO_VERSION)
  return resolver ? resolver.want(kind, id) : undefined
}

const KIND_MARK: Record<ReferenceKind, string> = { TICKET: '◆', DOC: '▤', RECORD: '▦', USER: '@' }

/**
 * A reference to a Ticket, Doc or Record. The node stores ids only; a viewer who may not read the
 * target sees a neutral chip with no title and no hint why.
 */
export function ReferenceChip({ kind, id }: { kind: ReferenceKind; id: string }) {
  const { t } = useI18n()
  const { open } = useDocContext()
  const resolved = useResolved(kind, id)
  const loading = resolved === undefined
  const unavailable = resolved !== undefined && !resolved.available
  const label = loading ? '…' : unavailable ? t('docs.ref.unavailable') : (resolved.title ?? '')
  return (
    <span
      className={['doc-chip', `doc-chip-${kind.toLowerCase()}`, unavailable ? 'is-unavailable' : ''].filter(Boolean).join(' ')}
      data-reference-kind={kind}
      data-reference-id={id}
      contentEditable={false}
      role={unavailable ? undefined : 'link'}
      tabIndex={unavailable || loading ? undefined : 0}
      onClick={() => {
        if (resolved?.available) open(kind, id, { projectId: resolved.projectId })
      }}
      onKeyDown={(event) => {
        if (event.key === 'Enter' && resolved?.available) open(kind, id, { projectId: resolved.projectId })
      }}
    >
      <span aria-hidden="true">{KIND_MARK[kind]}</span> {label}
    </span>
  )
}

export function MentionChip({ userId }: { userId: string }) {
  const { t } = useI18n()
  const resolved = useResolved('USER', userId)
  const label = resolved === undefined ? '…' : resolved.available ? (resolved.title ?? '') : t('docs.ref.unavailable')
  return (
    <span className="doc-chip doc-chip-user" data-mention-user={userId} contentEditable={false}>
      @{label}
    </span>
  )
}
