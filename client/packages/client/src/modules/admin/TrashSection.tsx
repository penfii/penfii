import { useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Button, useConfirm } from '../../design/ui'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { compareKeys, matchesText } from '../work/listTools'
import { useAdminAction } from './useAdminAction'

type Sort = 'recent' | 'purge' | 'key'

/**
 * A Project's trash, for the Project admin: when each Ticket is purged automatically (or why that is on
 * hold), whether it was archived, restore, and purge for good (`canPurge`: the server lets only the Project's admin do it). Search and sort work on the
 * loaded list.
 */
export function TrashSection({ projectId, canPurge }: { projectId: string; canPurge: boolean }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const trash = useQuery(queries.projectTrash(projectId))
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<Sort>('recent')
  const [archivedOnly, setArchivedOnly] = useState(false)
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectTrash(projectId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectArchive(projectId) })
  }
  const all = trash.data
  const retentionDays = useMemo(() => {
    const sample = all?.find((ticket) => ticket.purgeAt)
    return sample?.purgeAt ? Math.round((Date.parse(sample.purgeAt) - Date.parse(sample.trashedAt)) / 86_400_000) : null
  }, [all])
  const shown = useMemo(() => {
    const rows = (all ?? []).filter((ticket) => (!archivedOnly || ticket.archived) && matchesText(ticket.key, ticket.title, query))
    const far = Number.MAX_SAFE_INTEGER
    const by = {
      recent: (a: (typeof rows)[number], b: (typeof rows)[number]) => b.trashedAt.localeCompare(a.trashedAt),
      purge: (a: (typeof rows)[number], b: (typeof rows)[number]) => (a.purgeAt ? Date.parse(a.purgeAt) : far) - (b.purgeAt ? Date.parse(b.purgeAt) : far),
      key: (a: (typeof rows)[number], b: (typeof rows)[number]) => compareKeys(a.key, b.key),
    }[sort]
    return [...rows].sort(by)
  }, [all, query, sort, archivedOnly])

  return (
    <section className="permissions" aria-labelledby="trash-title">
      <h3 id="trash-title">{t('admin.trash.title')}</h3>
      <p className="hint">{retentionDays ? t('admin.trash.leadRetention', { days: String(retentionDays) }) : t('admin.trash.lead')}</p>
      {all && all.length > 0 && !retentionDays ? <p className="hint">{t('admin.trash.purgeOff')}</p> : null}
      {trash.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {trash.isError ? <p className="board-error" role="alert">{t('admin.loadError')}</p> : null}
      {error ? <p className="board-error" role="alert">{error}</p> : null}
      {all && all.length === 0 ? <p className="hint">{t('admin.trash.empty')}</p> : null}
      {all && all.length > 0 ? (
        <>
          <div className="list-tools">
            <input type="search" aria-label={t('admin.trash.search')} placeholder={t('admin.trash.searchPlaceholder')} value={query} onChange={(event) => setQuery(event.target.value)} />
            <label className="view-select">
              <span>{t('admin.trash.sort')}</span>
              <select value={sort} onChange={(event) => setSort(event.target.value as Sort)}>
                <option value="recent">{t('admin.trash.sort.recent')}</option>
                <option value="purge">{t('admin.trash.sort.purge')}</option>
                <option value="key">{t('admin.trash.sort.key')}</option>
              </select>
            </label>
            <label className="filter-choice">
              <input type="checkbox" checked={archivedOnly} onChange={(event) => setArchivedOnly(event.target.checked)} />
              <span>{t('admin.trash.filter.archived')}</span>
            </label>
          </div>
          {shown.length === 0 ? <p className="hint">{t('admin.trash.noMatch')}</p> : null}
          <ul className="plain-list">
            {shown.map((ticket) => (
              <li key={ticket.id} className="row-actions trash-row">
                <span>
                  <strong>{ticket.key}</strong> {ticket.title}
                  {ticket.archived ? <>{' '}<span className="where-chip">{t('admin.trash.archivedBadge')}</span></> : null}
                </span>
                <small className="hint">{t('admin.trash.by', { name: ticket.trashedByName ?? '', when: formatStamp(ticket.trashedAt, locale) })}</small>
                {ticket.purgeBlocked ? (
                  <small className="board-error trash-held">
                    {ticket.purgeAt ? `${t('admin.trash.held')}: ` : ''}{t(ticket.purgeBlocked === 'children' ? 'admin.trash.hasChildren' : 'admin.trash.hasDependencies')}
                  </small>
                ) : ticket.purgeAt ? (
                  <small className="hint">{t('admin.trash.purgeAt', { when: formatStamp(ticket.purgeAt, locale) })}</small>
                ) : null}
                {ticket.archived ? <small className="hint">{t('admin.trash.archivedNote')}</small> : null}
                <Button variant="secondary" disabled={busy} aria-label={`${t('admin.trash.restore')} ${ticket.key}`} onClick={() => void perform(() => api.restoreTicketFromTrash(ticket.id), refresh)}>
                  {t('admin.trash.restore')}
                </Button>
                {canPurge ? (
                  <Button
                    variant="ghost"
                    disabled={busy || Boolean(ticket.purgeBlocked)}
                    title={ticket.purgeBlocked ? t(ticket.purgeBlocked === 'children' ? 'admin.trash.hasChildren' : 'admin.trash.hasDependencies') : undefined}
                    aria-label={`${t('admin.trash.purge')} ${ticket.key}`}
                    onClick={() => {
                      void confirm({ title: t('admin.trash.purge'), body: t('admin.trash.purgeConfirm', { key: ticket.key }), confirmLabel: t('admin.trash.purge') }).then((ok) => {
                        if (ok) void perform(() => api.purgeTicket(ticket.id), refresh)
                      })
                    }}
                  >
                    {t('admin.trash.purge')}
                  </Button>
                ) : null}
              </li>
            ))}
          </ul>
        </>
      ) : null}
      {dialog}
      {confirmDialog}
    </section>
  )
}
