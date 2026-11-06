import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Team, TeamRole, WorkspacePerson } from '@penfii/contracts'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { Button, Dialog, Field, Select } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { TEAM_ROLES } from './TeamMembersPanel'
import { refreshTeamData } from './teamState'

type Outcome = { state: 'added' | 'skipped' } | { state: 'failed'; reason: string }

/**
 * Adds the selected people to one Team, one by one with the existing per-member call. Each person has
 * their own result; a failure does not undo the others and can be retried. Not all-or-nothing.
 */
export function BatchAssignDialog({ open, workspaceId, people, teams, onClose, onDone }: {
  open: boolean
  workspaceId: string
  people: WorkspacePerson[]
  teams: Team[]
  onClose: () => void
  onDone: () => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [teamId, setTeamId] = useState('')
  const [role, setRole] = useState<TeamRole>('MEMBER')
  const [results, setResults] = useState<Record<string, Outcome>>({})
  const [running, setRunning] = useState(false)
  const team = teams.find((item) => item.id === teamId) ?? teams[0]
  const projects = useQuery({ ...queries.teamProjects(workspaceId, team?.id ?? ''), enabled: open && Boolean(team) })
  const withRole = (projects.data ?? []).filter((project) => project.teamRole !== null)
  const started = Object.keys(results).length > 0

  function reasonOf(caught: unknown): string {
    if (isApiError(caught) && caught.code === 'team_archived') return t('batch.reason.archived')
    if (isApiError(caught) && caught.status === 403) return t('batch.reason.forbidden')
    if (isApiError(caught) && caught.status === 404) return t('batch.reason.disabled')
    return t('batch.reason.generic')
  }

  async function run(only?: string[]) {
    if (!team || running) return
    setRunning(true)
    const next = { ...results }
    for (const person of people) {
      if (only && !only.includes(person.userId)) continue
      if (person.teams.some((item) => item.id === team.id)) {
        next[person.userId] = { state: 'skipped' }
        setResults({ ...next })
        continue
      }
      try {
        await api.addTeamMember(workspaceId, team.id, person.userId, role)
        next[person.userId] = { state: 'added' }
      } catch (caught) {
        next[person.userId] = { state: 'failed', reason: reasonOf(caught) }
      }
      setResults({ ...next })
    }
    setRunning(false)
    await refreshTeamData(queryClient, workspaceId)
    onDone()
  }

  const failedIds = Object.entries(results).filter(([, outcome]) => outcome.state === 'failed').map(([id]) => id)
  function close() {
    setResults({})
    setTeamId('')
    onClose()
  }

  return (
    <Dialog open={open} title={t('batch.title', { count: String(people.length) })} onClose={close} presentation="full">
      {teams.length === 0 ? <p className="hint">{t('batch.noTeams')}</p> : (
        <>
          <Field label={t('batch.team')}>
            <Select value={team?.id ?? ''} disabled={started || running} onChange={(event) => setTeamId(event.target.value)}>
              {teams.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
            </Select>
          </Field>
          {team?.description ? <p className="hint team-description">{team.description}</p> : null}
          <Field label={t('batch.role')}>
            <Select value={role} disabled={started || running} onChange={(event) => setRole(event.target.value as TeamRole)}>
              {TEAM_ROLES.map((item) => <option key={item} value={item}>{t(`team.role.${item}`)}</option>)}
            </Select>
          </Field>
          <div className="callout" role="status">
            {projects.isPending ? <p className="hint">{t('loading')}</p> : null}
            {projects.data && withRole.length > 0 ? (
              <>
                <strong>{t('batch.projectsTitle', { count: String(withRole.length) })}</strong>
                <p className="hint">{withRole.map((project) => `${project.name} (${t(`permissions.role.${project.teamRole!}`)})`).join(', ')}. {t('batch.projectsNote')}</p>
              </>
            ) : null}
            {projects.data && withRole.length === 0 ? <p className="hint">{t('batch.projectsNone')}</p> : null}
          </div>
          <ul className="plain-list batch-people">
            {people.map((person) => {
              const outcome = results[person.userId]
              return (
                <li key={person.userId} className="batch-person">
                  <span>{person.displayName}</span>
                  {outcome ? (
                    <span className={outcome.state === 'failed' ? 'form-error' : 'hint'} role={outcome.state === 'failed' ? 'alert' : undefined}>
                      {outcome.state === 'added' ? t('batch.added') : outcome.state === 'skipped' ? t('batch.skipped') : outcome.state === 'failed' ? t('batch.failed', { reason: outcome.reason }) : ''}
                    </span>
                  ) : person.teams.some((item) => item.id === team?.id) ? <span className="hint">{t('batch.skipped')}</span> : null}
                </li>
              )
            })}
          </ul>
        </>
      )}
      <div className="dialog-actions">
        <Button type="button" variant="secondary" onClick={close}>{t('batch.close')}</Button>
        {failedIds.length > 0 && !running ? <Button type="button" variant="secondary" onClick={() => void run(failedIds)}>{t('batch.retry')}</Button> : null}
        {!started && teams.length > 0 ? (
          <Button type="button" disabled={running || !team} onClick={() => void run()}>
            {running ? t('batch.running') : t('batch.run', { count: String(people.filter((person) => !person.teams.some((item) => item.id === team?.id)).length) })}
          </Button>
        ) : null}
      </div>
    </Dialog>
  )
}
