import { useState } from 'react'
import { Link, useNavigate } from 'react-router'
import type { WorkspacePerson } from '@penfii/contracts'
import { Avatar, Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { openPendingDm } from '../../shell/pendingDm'
import { useSession } from '../identity'

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
        {person.teams.length > 0 ? (
          <ul className="plain-list">
            {person.teams.map((team) => (
              <li key={team.id}><Link to={`/w/${workspaceId}/directory/teams/${team.id}`} onClick={() => setOpen(false)}>{team.name}</Link></li>
            ))}
          </ul>
        ) : <p className="hint">{t('teams.unaffiliated')}</p>}
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
