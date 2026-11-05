import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import { Avatar, Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { Popover } from '../work/Popover'
import { useSession } from '../identity'
import { queries } from '../../queries'
import { useStartDm } from '../../search/DirectoryResults'

/** The name on a message. Opening it shows the person's profile card and can start a direct message. */
export function AuthorCard({ workspaceId, userId, name }: { workspaceId: string; userId: string; name: string }) {
  if (!workspaceId) return <strong>{name}</strong>
  return (
    <Popover label={name} className="author-link">
      {(close) => <AuthorBody workspaceId={workspaceId} userId={userId} name={name} onClose={close} />}
    </Popover>
  )
}

function AuthorBody({
  workspaceId,
  userId,
  name,
  onClose,
}: {
  workspaceId: string
  userId: string
  name: string
  onClose: () => void
}) {
  const { t } = useI18n()
  const session = useSession()
  const people = useQuery({ ...queries.people(workspaceId), enabled: workspaceId.length > 0 })
  const person = people.data?.find((item) => item.userId === userId)
  const dm = useStartDm(workspaceId, onClose)
  const shown = person?.displayName ?? name
  const status = [person?.statusEmoji, person?.statusText].filter(Boolean).join(' ')
  const self = session.data?.id === userId
  return (
    <div className="person-card">
      <div className="menu-identity">
        <Avatar name={shown} userId={userId} hasAvatar={person?.hasAvatar} />
        <span>
          <strong>{shown}</strong>
          {person?.jobTitle ? <small>{person.jobTitle}</small> : null}
          {status ? <small className="hint">{status}</small> : null}
          {person?.email ? <small className="hint">{person.email}</small> : null}
        </span>
      </div>
      {person && person.teams.length > 0 ? (
        <ul className="plain-list">
          {person.teams.map((team) => (
            <li key={team.id}>
              <Link to={`/w/${workspaceId}/directory/teams/${team.id}`} onClick={onClose}>{team.name}</Link>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="dialog-actions">
        {self ? (
          <Link to={`/w/${workspaceId}/settings`} onClick={onClose}>{t('settings.profile')}</Link>
        ) : (
          <Button type="button" disabled={dm.busy} onClick={() => void dm.open([userId])}>{t('teams.message')}</Button>
        )}
      </div>
    </div>
  )
}
