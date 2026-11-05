import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { ProjectPermissions, ProjectRole } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Select } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'

const ROLES: ProjectRole[] = ['ADMIN', 'EDITOR', 'VIEWER']

type Mode = 'whole' | 'picked'

/**
 * Invites people from a Team in one of two ways that mean different things. The whole Team is Team-derived
 * access: its current and future members come in with the role chosen here for as long as they are in the Team.
 * Picked members get direct grants of their own that the Team no longer touches.
 */
export function AddFromTeam({
  projectId,
  workspaceTeams,
  data,
  busy,
  run,
}: {
  projectId: string
  workspaceTeams: Array<{ id: string; name: string }>
  data: ProjectPermissions
  busy: boolean
  run: (change: () => Promise<void>) => void
}) {
  const { t } = useI18n()
  const [mode, setMode] = useState<Mode>('whole')
  const [teamId, setTeamId] = useState('')
  const [role, setRole] = useState<ProjectRole>('VIEWER')
  const [picked, setPicked] = useState<string[]>([])
  const linked = new Set(data.teams.map((team) => team.teamId))
  // Individual people can be picked from a Team that is linked to the Project; the whole Team can be any Team.
  const choices = mode === 'whole' ? workspaceTeams : workspaceTeams.filter((team) => linked.has(team.id))
  const chosen = choices.some((team) => team.id === teamId) ? teamId : ''
  const roster = useQuery({ ...queries.projectTeamMembers(projectId, chosen), enabled: mode === 'picked' && chosen !== '' })
  const direct = new Set(data.members.map((member) => member.userId))
  const selectable = (roster.data ?? []).filter((member) => !direct.has(member.userId))

  return (
    <fieldset className="add-from-team">
      <legend>{t('permissions.fromTeam')}</legend>
      <label>
        <input type="radio" name={`mode-${projectId}`} checked={mode === 'whole'} onChange={() => { setMode('whole'); setPicked([]) }} /> {t('permissions.fromTeam.whole')}
      </label>
      <p className="hint">{t('permissions.fromTeam.wholeHint')}</p>
      <label>
        <input type="radio" name={`mode-${projectId}`} checked={mode === 'picked'} onChange={() => { setMode('picked'); setPicked([]) }} /> {t('permissions.fromTeam.picked')}
      </label>
      <p className="hint">{t('permissions.fromTeam.pickedHint')}</p>
      <div className="row-actions">
        <Select aria-label={t('permissions.fromTeam.team')} value={chosen} onChange={(event) => { setTeamId(event.target.value); setPicked([]) }}>
          <option value="">—</option>
          {choices.map((team) => <option key={team.id} value={team.id}>{team.name}</option>)}
        </Select>
        <Select aria-label={t('permissions.fromTeam.role')} value={role} onChange={(event) => setRole(event.target.value as ProjectRole)}>
          {ROLES.map((item) => <option key={item} value={item}>{t(`permissions.role.${item}`)}</option>)}
        </Select>
      </div>
      {mode === 'picked' && chosen !== '' ? (
        <ul className="plain-list">
          {roster.data && selectable.length === 0 ? <li className="hint">{t('permissions.fromTeam.none')}</li> : null}
          {selectable.map((member) => (
            <li key={member.userId}>
              <label>
                <input
                  type="checkbox"
                  checked={picked.includes(member.userId)}
                  onChange={(event) => setPicked((current) => (event.target.checked ? [...current, member.userId] : current.filter((id) => id !== member.userId)))}
                />{' '}
                {member.displayName}
              </label>
            </li>
          ))}
        </ul>
      ) : null}
      {mode === 'whole' ? (
        <Button
          variant="secondary"
          disabled={busy || chosen === ''}
          onClick={() => {
            const team = chosen
            // A Team that is not linked yet is linked first; the role is then the Team's participation.
            run(async () => {
              if (!linked.has(team)) await api.linkProjectTeam(projectId, team)
              await api.setProjectTeamRole(projectId, team, role)
            })
            setTeamId('')
          }}
        >
          {t('permissions.fromTeam.inviteWhole')}
        </Button>
      ) : (
        <Button
          variant="secondary"
          disabled={busy || picked.length === 0}
          onClick={() => {
            const people = picked
            run(async () => {
              for (const userId of people) await api.setProjectGrant(projectId, userId, role)
            })
            setPicked([])
          }}
        >
          {t('permissions.fromTeam.addPicked')}
        </Button>
      )}
    </fieldset>
  )
}
