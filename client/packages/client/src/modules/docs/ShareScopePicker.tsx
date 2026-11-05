import { activeTeams } from '../spaces/teamState'
import { useQuery } from '@tanstack/react-query'
import type { DocVisibility, Space } from '@penfii/contracts'
import { Select } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { queries } from '../../queries'

/** Who something is shared with. Where it was made is a separate thing and is not part of this. */
export type ShareScope = { visibility: DocVisibility; projectId: string | null; teamId: string | null }

type Translate = (key: MessageKey, params?: Record<string, string>) => string

/** "Whole Workspace", "Project Alpha", "Design of Project Alpha", or "Invited people only": what the person sees as the current scope. */
export function scopeLabel(scope: ShareScope, space: Space | undefined, t: Translate): string {
  if (scope.visibility === 'WORKSPACE') return t('docs.scope.workspace')
  if (scope.visibility === 'RESTRICTED') return t('docs.scope.private')
  const project = space?.projects.find((item) => item.id === scope.projectId)?.name ?? t('docs.scope.project')
  const team = scope.teamId ? space?.teams.find((item) => item.id === scope.teamId)?.name : null
  const name = space?.projects.find((item) => item.id === scope.projectId)?.name
  if (!name) return project
  return team ? t('docs.scope.projectTeam', { project: name, team }) : t('docs.scope.projectNamed', { project: name })
}

/**
 * The share scope choices: the Workspace, a Project (optionally narrowed to one of its Teams: the people who
 * are in both), or only the people invited by name. A Team alone is not a choice.
 */
export function ShareScopePicker({
  workspaceId,
  value,
  onChange,
  disabled = false,
  name,
  suggestedProjectId = null,
}: {
  workspaceId: string
  value: ShareScope
  onChange: (next: ShareScope) => void
  disabled?: boolean
  name: string
  /** The Project the page is being made in: offered first when the person picks "a Project". */
  suggestedProjectId?: string | null
}) {
  const { t } = useI18n()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  // A page is shared into a Project only by its participants; the one it is already shared with stays listed.
  const projects = (space.data?.projects ?? []).filter((item) => item.participating !== false || item.id === value.projectId)
  const project = projects.find((item) => item.id === value.projectId)
  const teams = activeTeams(space.data?.teams ?? []).filter((team) => project?.teamIds.includes(team.id))
  const pick = (visibility: DocVisibility) => {
    if (visibility !== 'PROJECT') return onChange({ visibility, projectId: null, teamId: null })
    const first = value.projectId ?? suggestedProjectId ?? projects[0]?.id ?? null
    onChange({ visibility, projectId: first, teamId: null })
  }
  return (
    <fieldset disabled={disabled} className="share-scope">
      <legend>{t('docs.access.visibility')}</legend>
      <label>
        <input type="radio" name={name} checked={value.visibility === 'WORKSPACE'} onChange={() => pick('WORKSPACE')} /> {t('docs.access.workspace')}
      </label>
      <label>
        <input type="radio" name={name} checked={value.visibility === 'PROJECT'} onChange={() => pick('PROJECT')} disabled={projects.length === 0} /> {t('docs.access.project')}
      </label>
      {value.visibility === 'PROJECT' ? (
        <div className="share-scope-project">
          <label>
            <span>{t('docs.access.projectName')}</span>
            <Select value={value.projectId ?? ''} onChange={(event) => onChange({ ...value, projectId: event.target.value || null, teamId: null })}>
              {projects.map((item) => (
                <option key={item.id} value={item.id}>{item.name}</option>
              ))}
            </Select>
          </label>
          <label>
            <span>{t('docs.access.projectTeam')}</span>
            <Select value={value.teamId ?? ''} onChange={(event) => onChange({ ...value, teamId: event.target.value || null })}>
              <option value="">{t('docs.access.teamAll')}</option>
              {teams.map((team) => (
                <option key={team.id} value={team.id}>{team.name}</option>
              ))}
            </Select>
          </label>
        </div>
      ) : null}
      <label>
        <input type="radio" name={name} checked={value.visibility === 'RESTRICTED'} onChange={() => pick('RESTRICTED')} /> {t('docs.access.restricted')}
      </label>
      <p className="hint">
        {value.visibility === 'RESTRICTED'
          ? t('docs.access.restrictedHint')
          : value.visibility === 'PROJECT'
            ? value.teamId
              ? t('docs.access.projectTeamHint')
              : t('docs.access.projectHint')
            : t('docs.access.workspaceHint')}
      </p>
    </fieldset>
  )
}
