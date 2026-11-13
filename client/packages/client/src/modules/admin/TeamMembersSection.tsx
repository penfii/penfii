import { useState } from 'react'
import { QueryError } from '../../ui/QueryError'
import { useQuery } from '@tanstack/react-query'
import { Select } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { TeamMembersPanel } from '../spaces/TeamMembersPanel'

export function TeamMembersSection({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const space = useQuery(queries.space(workspaceId))
  const [picked, setPicked] = useState('')
  const teams = space.data?.teams ?? []
  const teamId = teams.some((team) => team.id === picked) ? picked : (teams[0]?.id ?? '')

  return (
    <div className="card settings-card" aria-labelledby="admin-teams-title">
      <h3 id="admin-teams-title">{t('admin.teams.title')}</h3>
      {space.isError ? <QueryError query={space} compact /> : null}
      {space.isSuccess && teams.length === 0 ? <p className="hint">{t('admin.teams.none')}</p> : null}
      {teams.length > 0 ? (
        <>
          <label className="field">
            <span>{t('admin.teams.pick')}</span>
            <Select value={teamId} onChange={(event) => setPicked(event.target.value)}>
              {teams.map((team) => <option key={team.id} value={team.id}>{team.name}</option>)}
            </Select>
          </label>
          <TeamMembersPanel key={teamId} workspaceId={workspaceId} teamId={teamId} archived={Boolean(teams.find((team) => team.id === teamId)?.archivedAt)} canManage showWorkspaceRole />
        </>
      ) : null}
    </div>
  )
}
