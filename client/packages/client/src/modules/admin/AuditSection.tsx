import { useState } from 'react'
import { useInfiniteQuery } from '@tanstack/react-query'
import type { AuditRecord } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Select } from '../../design/ui'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { queryKeys } from '../../queryKeys'

function roleName(value: unknown, t: ReturnType<typeof useI18n>['t']): string {
  return value === 'OWNER' || value === 'ADMIN' || value === 'MEMBER' ? t(`admin.role.${value}`) : String(value ?? '')
}

/** Who or what an entry is about: the target, the person it concerns, and a role change's before and after. */
function targetText(entry: AuditRecord, t: ReturnType<typeof useI18n>['t']): string {
  const parts = [entry.targetLabel, entry.subjectLabel].filter((part): part is string => Boolean(part))
  if (entry.action === 'member.role_changed') {
    parts.push(t('admin.audit.roleChange', { before: roleName(entry.data.before, t), after: roleName(entry.data.after, t) }))
  }
  return parts.join(' · ')
}

const GROUPS = ['member.', 'invitation.', 'team.', 'user.', 'ticket.'] as const

export function AuditSection({ workspaceId }: { workspaceId: string }) {
  const { t, locale, has } = useI18n()
  const [group, setGroup] = useState('')
  const audit = useInfiniteQuery({
    queryKey: queryKeys.adminAudit(workspaceId, group),
    queryFn: ({ pageParam }) => api.listAudit(workspaceId, { action: group || undefined, cursor: pageParam, limit: 25 }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  })
  const items = audit.data?.pages.flatMap((page) => page.items) ?? []

  return (
    <div className="card settings-card" aria-labelledby="admin-audit-title">
      <h3 id="admin-audit-title">{t('admin.audit.title')}</h3>
      <p className="field-hint">{t('admin.audit.lead')}</p>
      <label className="field">
        <span>{t('admin.audit.filter')}</span>
        <Select value={group} onChange={(event) => setGroup(event.target.value)}>
          <option value="">{t('admin.audit.all')}</option>
          {GROUPS.map((prefix) => <option key={prefix} value={prefix}>{t(`admin.audit.group.${prefix}`)}</option>)}
        </Select>
      </label>
      {audit.isError ? <p className="form-error" role="alert">{t('admin.loadError')}</p> : null}
      {audit.isSuccess && items.length === 0 ? <p className="hint">{t('admin.audit.empty')}</p> : null}
      {items.length > 0 ? (
        <div className="admin-table-wrap"><table className="record-table permissions-table">
          <thead>
            <tr>
              <th>{t('admin.audit.when')}</th>
              <th>{t('admin.audit.who')}</th>
              <th>{t('admin.audit.what')}</th>
              <th>{t('admin.audit.target')}</th>
            </tr>
          </thead>
          <tbody>
            {items.map((entry) => {
              const key = `admin.audit.action.${entry.action}` as MessageKey
              return (
                <tr key={entry.id}>
                  <td data-label={t('admin.audit.when')}>{formatStamp(entry.occurredAt, locale)}</td>
                  <td data-label={t('admin.audit.who')}>{entry.actorName ?? t('admin.audit.system')}</td>
                  <td data-label={t('admin.audit.what')}>{has(key) ? t(key) : <code>{entry.action}</code>}</td>
                  <td data-label={t('admin.audit.target')}>{targetText(entry, t)}</td>
                </tr>
              )
            })}
          </tbody>
        </table></div>
      ) : null}
      {audit.hasNextPage ? (
        <Button variant="secondary" disabled={audit.isFetchingNextPage} onClick={() => void audit.fetchNextPage()}>
          {t('admin.audit.more')}
        </Button>
      ) : null}
    </div>
  )
}
