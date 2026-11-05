import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Button, useConfirm } from '../../design/ui'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useAdminAction } from './useAdminAction'

/**
 * A Project's trash, for the Project admin: restore a Ticket, or (Workspace owners and admins only)
 * purge it for good.
 */
export function TrashSection({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const trash = useQuery(queries.projectTrash(projectId))
  const workspaces = useQuery(queries.workspaces())
  const role = workspaces.data?.find((workspace) => workspace.id === workspaceId)?.role
  const canPurge = role === 'OWNER' || role === 'ADMIN'
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectTrash(projectId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.projectBoard(projectId) })
  }

  return (
    <section className="permissions" aria-labelledby="trash-title">
      <h3 id="trash-title">{t('admin.trash.title')}</h3>
      <p className="hint">{t('admin.trash.lead')}</p>
      {trash.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {trash.isError ? <p className="board-error" role="alert">{t('admin.loadError')}</p> : null}
      {error ? <p className="board-error" role="alert">{error}</p> : null}
      {trash.data && trash.data.length === 0 ? <p className="hint">{t('admin.trash.empty')}</p> : null}
      {trash.data && trash.data.length > 0 ? (
        <ul className="plain-list">
          {trash.data.map((ticket) => (
            <li key={ticket.id} className="row-actions">
              <span><strong>{ticket.key}</strong> {ticket.title}</span>
              <small className="hint">{t('admin.trash.by', { name: ticket.trashedByName ?? '', when: formatStamp(ticket.trashedAt, locale) })}</small>
              <Button variant="secondary" disabled={busy} aria-label={`${t('admin.trash.restore')} ${ticket.key}`} onClick={() => void perform(() => api.restoreTicketFromTrash(ticket.id), refresh)}>
                {t('admin.trash.restore')}
              </Button>
              {canPurge ? (
                <Button
                  variant="ghost"
                  disabled={busy}
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
      ) : null}
      {dialog}
      {confirmDialog}
    </section>
  )
}
