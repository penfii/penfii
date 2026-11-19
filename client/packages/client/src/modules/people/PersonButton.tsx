import { useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import type { WorkspacePerson } from '@penfii/contracts'
import { Avatar, Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { openPendingDm } from '../../shell/pendingDm'
import { useSession } from '../identity'
import { queries } from '../../queries'
import { departmentPath } from '../spaces/orgTree'

/** A person's name that opens their card: job title, status, their Teams, and a message to them. */
export function PersonButton({ workspaceId, person }: { workspaceId: string; person: WorkspacePerson }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const session = useSession()
  const [open, setOpen] = useState(false)
  const status = [person.statusEmoji, person.statusText].filter(Boolean).join(' ')
  const self = session.data?.id === person.userId
  return (
    <>
      <button type="button" className="person-button" onClick={() => setOpen(true)}>
        <Avatar name={person.displayName} userId={person.userId} hasAvatar={person.hasAvatar} />
        <span>
          <strong>{person.displayName}</strong>
          {person.jobTitle ? <small> {person.jobTitle}</small> : null}
          {status ? <small className="hint"> {status}</small> : null}
          <small className="hint"> {person.email}</small>
        </span>
      </button>
      <Dialog open={open} title={person.displayName} onClose={() => setOpen(false)}>
        <div className="menu-identity">
          <Avatar name={person.displayName} userId={person.userId} hasAvatar={person.hasAvatar} />
          <span>
            {person.jobTitle ? <span>{person.jobTitle}</span> : null}
            {status ? <small className="hint">{status}</small> : null}
            <small className="hint">{person.email}</small>
          </span>
        </div>
        {open ? <PersonTeams workspaceId={workspaceId} person={person} onNavigate={() => setOpen(false)} /> : null}
        {self ? null : (
          <Button
            type="button"
            onClick={() => {
              setOpen(false)
              openPendingDm(navigate, workspaceId, [{ id: person.userId, displayName: person.displayName }])
            }}
          >
            {t('teams.message')}
          </Button>
        )}
      </Dialog>
    </>
  )
}

/**
 * The Teams a person is on, each with the department above it, a link to the Team and a link that finds the Team in the
 * org chart. A person may be on several Teams, so each one has its own pair of links. The two links are separate: opening a
 * Team and seeing where it sits are different questions.
 */
function PersonTeams({ workspaceId, person, onNavigate }: { workspaceId: string; person: WorkspacePerson; onNavigate: () => void }) {
  const { t } = useI18n()
  // Read only while the card is open.
  const space = useQuery(queries.space(workspaceId))
  const departments = useQuery(queries.departments(workspaceId))
  if (person.teams.length === 0) return <p className="hint">{t('teams.noTeam')}</p>
  return (
    <ul className="plain-list person-teams-list">
      {person.teams.map((team) => {
        const full = space.data?.teams.find((item) => item.id === team.id)
        const path = departmentPath(departments.data ?? [], full?.departmentId ?? null)
        return (
          <li key={team.id} className="person-team-row">
            <span>
              <Link to={`/w/${workspaceId}/directory/teams/${team.id}`} aria-label={t('teams.openTeamFor', { name: team.name })} onClick={onNavigate}>{team.name}</Link>
              <small className="hint"> {path.length > 0 ? path.join(' › ') : departments.isSuccess ? t('teams.unplacedTeam') : ''}</small>
            </span>
            <Link to={`/w/${workspaceId}/directory?focus=team:${team.id}`} aria-label={t('teams.viewInOrgFor', { name: team.name })} onClick={onNavigate}>{t('teams.viewInOrg')}</Link>
          </li>
        )
      })}
    </ul>
  )
}
