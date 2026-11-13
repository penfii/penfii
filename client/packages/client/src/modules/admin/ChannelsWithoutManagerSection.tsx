import { useState } from 'react'
import { QueryError } from '../../ui/QueryError'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import type { ChannelWithoutManager } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Select, useConfirm } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { useSession } from '../identity'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useAdminAction } from './useAdminAction'
import type { RecoveryStatus } from './recoveryStatus'

/**
 * Private channels nobody can manage (no manager who is an active member with an enabled account) and
 * the one thing a Workspace owner or admin may do about it: name a manager. Nothing is promoted on its
 * own, and the confirmation says that the person named gets to read the channel. Public channels are
 * not listed: they can do without a manager. The section is absent while every private channel has one.
 */
export function ChannelsWithoutManagerSection({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const channels = useQuery(queries.adminChannelsWithoutManager(workspaceId))
  const members = useQuery(queries.adminMembers(workspaceId))
  // The outcome is kept here, not in the row: naming the manager makes the row disappear.
  const [status, setStatus] = useState<RecoveryStatus | null>(null)
  // A failed read must not look like "nothing to recover".
  if (channels.isError) return <QueryError query={channels} compact />
  if (!channels.data?.length && !status) return null
  // Only people who can act: an active member whose account is not disabled.
  const candidates = (members.data ?? []).filter((member) => member.state === 'ACTIVE' && !member.accountDisabled)
  return (
    <div className="card settings-card" aria-labelledby="admin-no-manager-title">
      <h3 id="admin-no-manager-title">{t('admin.noManager.title')}</h3>
      {channels.data?.length ? <p className="field-hint">{t('admin.noManager.lead')}</p> : null}
      {status ? (
        <p className={status.tone === 'error' ? 'form-error' : 'hint'} role={status.tone === 'error' ? 'alert' : 'status'}>
          {t(status.key)}
        </p>
      ) : null}
      <ul className="plain-list">
        {(channels.data ?? []).map((channel) => (
          <ChannelRow key={channel.id} workspaceId={workspaceId} channel={channel} candidates={candidates} report={setStatus} />
        ))}
      </ul>
    </div>
  )
}

function ChannelRow({
  workspaceId,
  channel,
  candidates,
  report,
}: {
  workspaceId: string
  channel: ChannelWithoutManager
  candidates: { userId: string; displayName: string; email: string }[]
  report: (status: RecoveryStatus | null) => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const me = useSession()
  const { perform, busy, error, dialog } = useAdminAction()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const [chosen, setChosen] = useState('')

  async function assign() {
    const person = candidates.find((candidate) => candidate.userId === chosen)
    if (!person) return
    const self = person.userId === me.data?.id
    const body = self
      ? t('admin.noManager.confirmSelf', { channel: channel.name })
      : t('admin.noManager.confirm', { person: person.displayName, channel: channel.name })
    if (!(await confirm({ title: t('admin.noManager.assign'), body, confirmLabel: t('admin.noManager.assign'), tone: 'primary' }))) return
    report(null)
    let result: Awaited<ReturnType<typeof api.recoverChannelManager>> | null = null
    const ok = await perform(
      async () => {
        try {
          result = await api.recoverChannelManager(channel.id, person.userId)
        } catch (caught) {
          // Somebody named a manager first: the list is out of date, not the request wrong. Say so,
          // and read the list again so the row that no longer applies goes away.
          if (isApiError(caught) && (caught.code === 'channel_has_manager' || caught.code === 'channel_not_private')) {
            report({ tone: 'error', key: 'admin.noManager.hasManager' })
            void queryClient.invalidateQueries({ queryKey: queryKeys.adminChannelsWithoutManager(workspaceId) })
          }
          throw caught
        }
      },
      async () => {
        await queryClient.invalidateQueries({ queryKey: queryKeys.adminChannelsWithoutManager(workspaceId) })
        await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
      },
    )
    if (ok) report({ tone: 'ok', key: (result as { newAccess: boolean } | null)?.newAccess ? 'admin.noManager.doneNewAccess' : 'admin.noManager.done' })
  }

  const former = channel.formerManagers.map((manager) => manager.displayName).join(', ')
  return (
    <li className="admin-no-admin-row">
      <strong>{channel.name}</strong>{' '}
      <small className="hint">
        {channel.projectName
          ? t('admin.noManager.scope', { project: channel.projectName, count: String(channel.memberCount) })
          : t('admin.noManager.scopeGlobal', { count: String(channel.memberCount) })}
      </small>
      <p className="hint">{former ? t('admin.noManager.former', { names: former }) : t('admin.noManager.formerNone')}</p>
      <div className="row-actions">
        <Select aria-label={`${t('admin.noManager.choose')}: ${channel.name}`} value={chosen} disabled={busy} onChange={(event) => setChosen(event.target.value)}>
          <option value="">{t('admin.noManager.pick')}</option>
          {candidates.map((person) => (
            <option key={person.userId} value={person.userId}>{`${person.displayName} (${person.email})`}</option>
          ))}
        </Select>
        <Button disabled={busy || !chosen} aria-label={`${t('admin.noManager.assign')}: ${channel.name}`} onClick={() => void assign()}>
          {t('admin.noManager.assign')}
        </Button>
      </div>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
      {dialog}
      {confirmDialog}
    </li>
  )
}
