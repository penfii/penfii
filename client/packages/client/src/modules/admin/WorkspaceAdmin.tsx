import { useState } from 'react'
import { useAdminAction } from './useAdminAction'
import { QueryError } from '../../ui/QueryError'
import { NavLink } from 'react-router'
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
import { ProjectsWithoutAdminSection } from './ProjectsWithoutAdminSection'
import { ChannelsWithoutManagerSection } from './ChannelsWithoutManagerSection'
import { TeamMembersSection } from './TeamMembersSection'
import { McpPolicySettings } from '../identity'
import { ADMIN_SECTIONS, adminSectionLabel, adminSectionPath, type AdminSection } from './adminSections'

/** True for the roles that manage a Workspace. */
export function canAdminister(role: string | undefined): role is 'OWNER' | 'ADMIN' {
  return role === 'OWNER' || role === 'ADMIN'
}

/**
 * One page of Workspace administration (information, members, Team members, invitations, recovery,
 * integrations, audit log), under the Workspace's name so it is clear which one is being managed. Owners and admins only; the
 * server decides what each role may do, so these sections only hide the controls that role would be
 * refused.
 */
export function WorkspaceAdmin({ workspaceId, section = 'general' }: { workspaceId: string; section?: AdminSection }) {
  const { t } = useI18n()
  const workspaces = useQuery(queries.workspaces())
  if (workspaces.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (workspaces.isError) return <QueryError query={workspaces} />
  const workspace = workspaces.data?.find((item) => item.id === workspaceId)
  const role = workspace?.role
  if (!workspace || !canAdminister(role)) return <EmptyState title={t('admin.noAccess.title')} body={t('admin.noAccess.body')} />
  return (
    <section className="workspace-admin" aria-labelledby="workspace-admin-title">
      <h1 id="workspace-admin-title">{t('admin.pageTitle', { name: workspace.name })}</h1>
      <p className="field-hint">{t('admin.lead')}</p>
      <nav className="admin-subnav" aria-label={t('admin.sections')}>
        {ADMIN_SECTIONS.map((id) => (
          <NavLink key={id} end to={`/w/${workspaceId}/settings${adminSectionPath(id)}`}>{t(adminSectionLabel(id))}</NavLink>
        ))}
      </nav>
      {section === 'general' ? <WorkspaceFields workspace={workspace} /> : null}
      {section === 'members' ? <MembersSection workspaceId={workspaceId} viewerRole={role} /> : null}
      {section === 'teams' ? <TeamMembersSection workspaceId={workspaceId} /> : null}
      {section === 'invitations' ? <InvitationsSection workspaceId={workspaceId} viewerRole={role} /> : null}
      {section === 'recovery' ? (
        <>
          <p className="field-hint">{t('admin.recovery.lead')}</p>
          <ProjectsWithoutAdminSection workspaceId={workspaceId} />
          <ChannelsWithoutManagerSection workspaceId={workspaceId} />
        </>
      ) : null}
      {section === 'integrations' ? <McpPolicySettings workspaceId={workspaceId} whenOff={<p className="hint" role="status">{t('admin.integrations.off')}</p>} /> : null}
      {section === 'audit' ? <AuditSection workspaceId={workspaceId} /> : null}
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
  const { perform, busy, error, dialog } = useAdminAction()
  async function save() {
    if (busy || !name.trim()) return
    setSaved(false)
    // The same way every administrative call runs: a recent sign-in when the server asks, a readable failure, the lists read again.
    const done = await perform(
      () => api.updateWorkspace(workspace.id, { name: name.trim(), timeZone }),
      () => queryClient.invalidateQueries({ queryKey: queryKeys.workspaces }),
    )
    setSaved(done)
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
      {error ? <p className="field-error" role="alert">{error}</p> : null}
      {saved ? <p className="hint" role="status">{t('admin.workspace.saved')}</p> : null}
      <Button type="submit" disabled={busy || !name.trim()}>{t('admin.workspace.save')}</Button>
      {dialog}
    </form>
  )
}
