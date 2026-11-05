import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Workspace } from '@penfii/contracts'
import { api } from '../../api'
import { Button, EmptyState, Field } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { AuditSection } from './AuditSection'
import { InvitationsSection } from './InvitationsSection'
import { MembersSection } from './MembersSection'
import { TeamMembersSection } from './TeamMembersSection'

/** True for the roles that manage a Workspace. */
export function canAdminister(role: string | undefined): role is 'OWNER' | 'ADMIN' {
  return role === 'OWNER' || role === 'ADMIN'
}

/**
 * The Workspace administration page body: members, invitations, Team membership and the audit log,
 * under the Workspace's name so it is clear which one is being managed. Owners and admins only; the
 * server decides what each role may do, so these sections only hide the controls that role would be
 * refused.
 */
export function WorkspaceAdmin({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const workspaces = useQuery(queries.workspaces())
  if (workspaces.isPending) return <p className="hint" role="status">{t('loading')}</p>
  const workspace = workspaces.data?.find((item) => item.id === workspaceId)
  const role = workspace?.role
  if (!workspace || !canAdminister(role)) return <EmptyState title={t('admin.noAccess.title')} body={t('admin.noAccess.body')} />
  return (
    <section className="workspace-admin" aria-labelledby="workspace-admin-title">
      <h1 id="workspace-admin-title">{t('admin.pageTitle', { name: workspace.name })}</h1>
      <p className="field-hint">{t('admin.lead')}</p>
      <WorkspaceFields workspace={workspace} />
      <MembersSection workspaceId={workspaceId} viewerRole={role} />
      <InvitationsSection workspaceId={workspaceId} viewerRole={role} />
      <TeamMembersSection workspaceId={workspaceId} />
      <AuditSection workspaceId={workspaceId} />
    </section>
  )
}

function WorkspaceFields({ workspace }: { workspace: Workspace }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const zones = Intl.supportedValuesOf('timeZone')
  const [name, setName] = useState(workspace.name)
  const [timeZone, setTimeZone] = useState(workspace.timeZone)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState(false)
  const [busy, setBusy] = useState(false)
  async function save() {
    if (busy || !name.trim()) return
    setBusy(true)
    setError(false)
    setSaved(false)
    try {
      await api.updateWorkspace(workspace.id, { name: name.trim(), timeZone })
      await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces })
      setSaved(true)
    } catch {
      setError(true)
    } finally {
      setBusy(false)
    }
  }
  return (
    <form className="card settings-card" onSubmit={(event) => { event.preventDefault(); void save() }}>
      <h2>{t('admin.workspace.title')}</h2>
      <Field label={t('admin.workspace.name')}>
        <input aria-label={t('admin.workspace.name')} value={name} onChange={(event) => setName(event.target.value)} />
      </Field>
      <Field label={t('admin.workspace.timeZone')}>
        <select aria-label={t('admin.workspace.timeZone')} value={timeZone} onChange={(event) => setTimeZone(event.target.value)}>
          {zones.includes(timeZone) ? null : <option value={timeZone}>{timeZone}</option>}
          {zones.map((zone) => <option key={zone} value={zone}>{zone}</option>)}
        </select>
      </Field>
      {error ? <p className="field-error" role="alert">{t('admin.actionError')}</p> : null}
      {saved ? <p className="hint" role="status">{t('admin.workspace.saved')}</p> : null}
      <Button type="submit" disabled={busy || !name.trim()}>{t('admin.workspace.save')}</Button>
    </form>
  )
}
