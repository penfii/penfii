import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Button } from '../../design/ui'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { compareKeys, matchesText } from './listTools'

type Sort = 'recent' | 'oldest' | 'key'
const KINDS = ['EPIC', 'TASK', 'SUBTASK'] as const

/**
 * The Project's archived Tickets: readable by anyone who can read the Project, opened read-only in the
 * panel. Search, kind and sort work on the loaded list (the server returns up to 500, newest first).
 */
export function ArchivedTickets({ projectId, onOpen, onClose }: { projectId: string; onOpen: (ticketId: string) => void; onClose: () => void }) {
  const { t, locale } = useI18n()
  const archive = useQuery(queries.projectArchive(projectId))
  const [query, setQuery] = useState('')
  const [kind, setKind] = useState('')
  const [sort, setSort] = useState<Sort>('recent')
  const all = archive.data
  const shown = useMemo(() => {
    const rows = (all ?? []).filter((ticket) => (kind === '' || ticket.hierarchyKind === kind) && matchesText(ticket.key, ticket.title, query))
    const order = { recent: (a: (typeof rows)[number], b: (typeof rows)[number]) => b.archivedAt.localeCompare(a.archivedAt), oldest: (a: (typeof rows)[number], b: (typeof rows)[number]) => a.archivedAt.localeCompare(b.archivedAt), key: (a: (typeof rows)[number], b: (typeof rows)[number]) => compareKeys(a.key, b.key) }[sort]
    return [...rows].sort(order)
  }, [all, query, kind, sort])
  return (
    <section className="archive-panel" aria-labelledby="archive-title">
      <header className="archive-head">
        <h3 id="archive-title">{t('archive.title')}</h3>
        <Button type="button" variant="ghost" onClick={onClose}>{t('archive.close')}</Button>
      </header>
      <p className="hint">{t('archive.lead')}</p>
      {archive.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {archive.isError ? <p className="board-error" role="alert">{t('archive.error.generic')}</p> : null}
      {all && all.length === 0 ? <p className="hint">{t('archive.empty')}</p> : null}
      {all && all.length > 0 ? (
        <>
          <div className="list-tools">
            <input type="search" aria-label={t('archive.search')} placeholder={t('archive.searchPlaceholder')} value={query} onChange={(event) => setQuery(event.target.value)} />
            <label className="view-select">
              <span>{t('archive.kind')}</span>
              <select value={kind} onChange={(event) => setKind(event.target.value)}>
                <option value="">{t('archive.kind.all')}</option>
                {KINDS.map((item) => <option key={item} value={item}>{t(`work.kind.${item}`)}</option>)}
              </select>
            </label>
            <label className="view-select">
              <span>{t('archive.sort')}</span>
              <select value={sort} onChange={(event) => setSort(event.target.value as Sort)}>
                <option value="recent">{t('archive.sort.recent')}</option>
                <option value="oldest">{t('archive.sort.oldest')}</option>
                <option value="key">{t('archive.sort.key')}</option>
              </select>
            </label>
            <small className="field-hint" role="status">{t('archive.count', { shown: String(shown.length), total: String(all.length) })}</small>
          </div>
          {shown.length === 0 ? <p className="hint">{t('archive.noMatch')}</p> : null}
          <ul className="plain-list">
            {shown.map((ticket) => (
              <li key={ticket.id} className="row-actions">
                <button type="button" className="link-button" onClick={() => onOpen(ticket.id)}>
                  <strong>{ticket.key}</strong> {ticket.title}
                </button>
                <small className="hint">{t('archive.by', { name: ticket.archivedByName ?? '', when: formatStamp(ticket.archivedAt, locale) })}</small>
                {ticket.trashed ? <small className="hint">{t('archive.trashedNote')}</small> : null}
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </section>
  )
}
