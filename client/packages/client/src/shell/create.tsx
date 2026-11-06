import { createContext, useContext, useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, isNotFound } from '@penfii/api-client'
import type { Space, Team, TeamRole } from '@penfii/contracts'
import { activeTeams } from '../modules/spaces/teamState'
import { Link, useNavigate } from 'react-router'
import { useSession } from '../modules/identity'
import { api } from '../api'
import { Button, Dialog, Field, Select } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { openPendingDm } from './pendingDm'
import { startedConversation } from './pendingDm'
import { errorText } from '../ui/errorText'
import { useTicketPrefix } from '../modules/work/useTicketPrefix'
import { PersonPicker } from '../modules/spaces/PersonPicker'
import { TEAM_ROLES } from '../modules/spaces/TeamMembersPanel'
import { visibilityHint, visibilityLabel } from '../modules/spaces/visibilityLabel'

export type CreateKind = 'team' | 'project' | 'channel' | 'dm' | 'group' | 'invite' | 'task' | 'document'

const CreateContext = createContext<((kind: CreateKind, projectId?: string) => void) | null>(null)

export function useCreate(): (kind: CreateKind, projectId?: string) => void {
  const open = useContext(CreateContext)
  if (!open) throw new Error('CreateProvider is required')
  return open
}

export function CreateProvider({
  workspaceId,
  space,
  children,
}: {
  workspaceId: string
  space: Space | undefined
  children: ReactNode
}) {
  const [request, setRequest] = useState<{ kind: CreateKind; projectId?: string } | null>(null)
  return (
    <CreateContext.Provider value={(kind, projectId) => setRequest({ kind, projectId })}>
      {children}
      <CreateDialogs
        workspaceId={workspaceId}
        space={space}
        kind={request?.kind ?? null}
        projectId={request?.projectId}
        onClose={() => setRequest(null)}
      />
    </CreateContext.Provider>
  )
}

function CreateDialogs({
  workspaceId,
  space,
  kind,
  projectId,
  onClose,
}: {
  workspaceId: string
  space: Space | undefined
  kind: CreateKind | null
  projectId?: string
  onClose: () => void
}) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [notice, setNotice] = useState<string | null>(null)
  const [inviteToken, setInviteToken] = useState<string | null>(null)
  const teams = space?.teams ?? []
  const offerTeams = activeTeams(teams)

  useEffect(() => {
    setNotice(null)
    setInviteToken(null)
  }, [kind])

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
  }

  async function saved(action: () => Promise<void>) {
    setNotice(null)
    try {
      await action()
      await refresh()
      onClose()
    } catch (caught) {
      setNotice(isNotFound(caught) ? t('notFound') : errorText(caught, t('error.generic')))
    }
  }

  const unavailable = kind === 'task' || kind === 'document'
  const title =
    kind === 'team'
      ? t('create.team')
      : kind === 'project'
        ? t('create.project')
        : kind === 'channel'
          ? t('create.channel')
      : kind === 'dm'
        ? t('shell.chat.newDm')
        : kind === 'group'
          ? t('shell.chat.newGroup')
          : kind === 'invite'
              ? t('create.invite')
              : kind === 'task'
                ? t('create.task')
                : kind === 'document'
                  ? t('create.document')
                  : ''

  return (
    <Dialog open={kind != null} title={title} onClose={onClose} presentation="full">
      {notice ? <p className="form-error">{notice}</p> : null}
      {unavailable ? (
        <>
          <p>{t('create.unavailableBody')}</p>
          <Button variant="secondary" onClick={onClose}>
            {t('message.cancel')}
          </Button>
        </>
      ) : null}
      {kind === 'team' ? (
        <TeamDialog
          workspaceId={workspaceId}
          onSubmit={(body) => void saved(() => api.createTeam(workspaceId, body).then(() => undefined))}
        />
      ) : null}
      {kind === 'project' ? (
        <ProjectDialog
          workspaceId={workspaceId}
          teams={offerTeams}
          onSubmit={(name, teamIds, ticketPrefix) => void saved(async () => {
            const project = await api.createProject(workspaceId, { name, teamIds, ticketPrefix })
            void navigate(`/w/${workspaceId}/projects/${project.id}`)
          })}
        />
      ) : null}
      {kind === 'channel' ? (
        <ChannelDialog
          projectId={projectId ?? null}
          space={space}
          onClose={onClose}
          onSubmit={(body) => void saved(async () => {
            const channel = await api.createChannel(workspaceId, body)
            if (body.projectId) void navigate(`/w/${workspaceId}/projects/${body.projectId}/chat/${channel.conversationId}`)
            else void navigate(`/w/${workspaceId}/c/${channel.conversationId}`)
          })}
        />
      ) : null}
      {kind === 'dm' || kind === 'group' ? (
        <PeopleDialog
          workspaceId={workspaceId}
          space={space}
          multiple={kind === 'group'}
          onClose={onClose}
        />
      ) : null}
      {kind === 'invite' ? (
        <InviteDialog
          workspaceId={workspaceId}
          token={inviteToken}
          teams={offerTeams}
          onSubmit={async (email, role, teamId) => {
            setNotice(null)
            setInviteToken(null)
            try {
              const created = await api.createInvitation(workspaceId, { email, role, teamId })
              setInviteToken(created.inviteToken ?? null)
              await queryClient.invalidateQueries({ queryKey: queryKeys.adminInvitations(workspaceId) })
              await queryClient.invalidateQueries({ queryKey: queryKeys.all.adminAudit(workspaceId) })
              return true
            } catch (caught) {
              if (isApiError(caught) && caught.code === 'already_member') setNotice(t('admin.invite.alreadyMember'))
              else if (isApiError(caught) && caught.code === 'team_archived') setNotice(t('invite.teamArchived'))
              else if (isApiError(caught) && caught.status === 403) setNotice(t('admin.invite.roleNotAllowed'))
              else if (isApiError(caught) && caught.status === 409) setNotice(t('account.exists'))
              else setNotice(isNotFound(caught) ? t('notFound') : errorText(caught, t('error.generic')))
              return false
            }
          }}
        />
      ) : null}
    </Dialog>
  )
}

/** A Team starts with at least one member, each with a role chosen here; the creator is in it only if added. */
function TeamDialog({ workspaceId, onSubmit }: { workspaceId: string; onSubmit: (body: { name: string; description?: string; members: Array<{ userId: string; role: TeamRole }> }) => void }) {
  const { t } = useI18n()
  const session = useSession()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [members, setMembers] = useState<Array<{ id: string; displayName: string; role: TeamRole }>>([])
  return (
    <form
      onSubmit={(event: FormEvent) => {
        event.preventDefault()
        if (members.some((member) => member.role === 'MANAGER')) {
          onSubmit({
            name: name.trim(),
            ...(description.trim() === '' ? {} : { description: description.trim() }),
            members: members.map((member) => ({ userId: member.id, role: member.role })),
          })
        }
      }}
    >
      <Field label={t('workspaces.name')}>
        <input required value={name} onChange={(event) => setName(event.target.value)} />
      </Field>
      <Field label={t('teams.description')}>
        <textarea rows={2} value={description} placeholder={t('team.descriptionPlaceholder')} onChange={(event) => setDescription(event.target.value)} />
      </Field>
      <fieldset className="field">
        <legend>{t('team.members')}</legend>
        {!members.some((member) => member.role === 'MANAGER') ? <p className="hint">{t('team.members.need')}</p> : null}
        <ul className="plain-list">
          {members.map((member) => (
            <li key={member.id} className="row-actions">
              <span>{member.displayName}</span>
              <Select
                aria-label={`${member.displayName}: ${t('admin.teams.role')}`}
                value={member.role}
                onChange={(event) => setMembers((current) => current.map((item) => (item.id === member.id ? { ...item, role: event.target.value as TeamRole } : item)))}
              >
                {TEAM_ROLES.map((role) => <option key={role} value={role}>{t(`team.role.${role}`)}</option>)}
              </Select>
              <Button type="button" variant="ghost" aria-label={`${t('team.members.remove')} ${member.displayName}`} onClick={() => setMembers((current) => current.filter((item) => item.id !== member.id))}>
                {t('team.members.remove')}
              </Button>
            </li>
          ))}
        </ul>
        <PersonPicker
          workspaceId={workspaceId}
          taken={new Set(members.map((member) => member.id))}
          label={t('teamPage.addMembers')}
          preferUnaffiliated
          onPick={(person) => setMembers((current) => [...current, { ...person, role: 'MEMBER' }])}
        />
        {session.data && !members.some((member) => member.id === session.data.id) ? (
          <div className="row-actions">
            <Button type="button" variant="secondary" onClick={() => setMembers((current) => [...current, { id: session.data.id, displayName: session.data.displayName, role: 'MEMBER' }])}>{t('team.addMe')}</Button>
            <span className="hint">{t('team.addMe.hint')}</span>
          </div>
        ) : null}
      </fieldset>
      <Button type="submit" disabled={!members.some((member) => member.role === 'MANAGER')}>{t('team.create')}</Button>
    </form>
  )
}

function ProjectDialog({
  workspaceId,
  teams,
  onSubmit,
}: {
  workspaceId: string
  teams: Array<{ id: string; name: string }>
  onSubmit: (name: string, teamIds: string[], ticketPrefix: string) => void
}) {
  const { t } = useI18n()
  const [name, setName] = useState('')
  const [teamIds, setTeamIds] = useState<string[]>([])
  const prefix = useTicketPrefix(workspaceId, name)
  return (
    <form
      onSubmit={(event: FormEvent) => {
        event.preventDefault()
        if (prefix.usable) onSubmit(name.trim(), teamIds, prefix.value.trim().toUpperCase())
      }}
    >
      <Field label={t('workspaces.name')}>
        <input required value={name} onChange={(event) => setName(event.target.value)} />
      </Field>
      <Field label={t('work.prefix.label')} hint={prefix.hint}>
        <input
          required
          value={prefix.value}
          maxLength={10}
          autoCapitalize="characters"
          aria-invalid={prefix.touched && !prefix.usable}
          onChange={(event) => prefix.edit(event.target.value)}
        />
      </Field>
      {prefix.value ? <p className="hint prefix-preview">{t('work.prefix.preview', { key: `${prefix.value.trim().toUpperCase()}-001` })}</p> : null}
      <fieldset className="field">
        <legend>{t('project.teams')}</legend>
        <p className="hint">{t('project.teams.hint')}</p>
        {teams.length === 0 ? <p className="hint">{t('channel.needParents')}</p> : null}
        {teams.map((team) => (
          <label key={team.id}>
            <input
              type="checkbox"
              checked={teamIds.includes(team.id)}
              onChange={(event) => {
                setTeamIds((current) => (event.target.checked ? [...current, team.id] : current.filter((id) => id !== team.id)))
              }}
            />{' '}
            {team.name}
          </label>
        ))}
      </fieldset>
      <Button type="submit" disabled={!prefix.usable}>{t('create.project.submit')}</Button>
    </form>
  )
}

function ChannelDialog({
  projectId,
  space,
  onClose,
  onSubmit,
}: {
  projectId: string | null
  space: Space | undefined
  onClose: () => void
  onSubmit: (body: { name: string; description: string | null; visibility: 'PUBLIC' | 'PRIVATE'; teamId: string | null; projectId: string | null }) => void
}) {
  const { t } = useI18n()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [visibility, setVisibility] = useState<'PUBLIC' | 'PRIVATE'>('PUBLIC')
  // A Team narrows a Project's channel to the people who are also in it; it is offered only inside a Project.
  const [teamId, setTeamId] = useState('')
  const teams = activeTeams(space?.teams ?? []).filter((team) => space?.projects.find((item) => item.id === projectId)?.teamIds.includes(team.id))
  return (
    <form
      onSubmit={(event: FormEvent) => {
        event.preventDefault()
        onSubmit({
          name: name.trim(),
          description: description.trim() === '' ? null : description.trim(),
          visibility,
          teamId: projectId && visibility === 'PUBLIC' && teamId ? teamId : null,
          projectId,
        })
      }}
    >
      <Field label={t('channel.name')}>
        <input required value={name} onChange={(event) => setName(event.target.value)} />
      </Field>
      <Field label={t('shell.channel.description')}>
        <input value={description} onChange={(event) => setDescription(event.target.value)} />
      </Field>
      <Field label={t('channel.visibility')}>
        <select value={visibility} onChange={(event) => setVisibility(event.target.value === 'PRIVATE' ? 'PRIVATE' : 'PUBLIC')}>
          <option value="PUBLIC">{visibilityLabel(t, 'PUBLIC', projectId !== null)}</option>
          <option value="PRIVATE">{visibilityLabel(t, 'PRIVATE', projectId !== null)}</option>
        </select>
        <small className="field-hint">{visibilityHint(t, projectId !== null)}</small>
      </Field>
      {projectId && teams.length > 0 && visibility === 'PUBLIC' ? (
        <Field label={t('channel.team')} hint={t('channel.team.hint')}>
          <select value={teamId} onChange={(event) => setTeamId(event.target.value)}>
            <option value="">{t('docs.access.teamAll')}</option>
            {teams.map((team) => (
              <option key={team.id} value={team.id}>{team.name}</option>
            ))}
          </select>
        </Field>
      ) : null}
      <div className="dialog-actions">
        <Button type="button" variant="secondary" onClick={onClose}>{t('message.cancel')}</Button>
        <Button type="submit">{t('shell.channel.create')}</Button>
      </div>
    </form>
  )
}

function PeopleDialog({
  workspaceId,
  space,
  multiple,
  onClose,
}: {
  workspaceId: string
  space: Space | undefined
  multiple: boolean
  onClose: () => void
}) {
  const { t } = useI18n()
  const session = useSession()
  const navigate = useNavigate()
  const [query, setQuery] = useState('')
  const trimmed = query.trim()
  const results = useQuery({ ...queries.directorySearch(workspaceId, trimmed, 'person'), enabled: workspaceId.length > 0 })
  const people = results.data?.results.flatMap((hit) => (hit.type === 'person' && hit.person ? [hit.person] : [])) ?? []
  const [picked, setPicked] = useState<Array<{ id: string; displayName: string }>>([])

  function open(peers: Array<{ id: string; displayName: string }>) {
    if (!session.data || peers.length === 0) return
    const started = startedConversation(space?.dms ?? [], [session.data.id, ...peers.map((peer) => peer.id)])
    if (started) void navigate(`/w/${workspaceId}/c/${started.conversationId}`)
    else openPendingDm(navigate, workspaceId, peers)
    onClose()
  }

  return (
    <form
      onSubmit={(event: FormEvent) => {
        event.preventDefault()
        if (!multiple || picked.length < 2) return
        open(picked)
      }}
    >
      <Field label={t('shell.chat.search')}>
        <input value={query} placeholder="bob" onChange={(event) => setQuery(event.target.value)} />
      </Field>
      <ul className="directory-list">
        {people.map((person) => (
          <li key={person.id}>
            <button
              type="button"
              className="directory-row"
              aria-pressed={multiple ? picked.some((item) => item.id === person.id) : undefined}
              onClick={() => {
                if (!session.data) return
                if (!multiple) {
                  open([{ id: person.id, displayName: person.displayName }])
                  return
                }
                setPicked((current) => (
                  current.some((item) => item.id === person.id)
                    ? current.filter((item) => item.id !== person.id)
                    : [...current, { id: person.id, displayName: person.displayName }]
                ))
              }}
            >
              <span>
                <strong>{person.displayName}</strong>
                <small>{person.username ? `@${person.username}` : person.email}</small>
              </span>
            </button>
          </li>
        ))}
      </ul>
      {results.isError ? <p className="form-error">{t('error.generic')}</p> : null}
      {multiple ? (
        <div className="dialog-actions">
          <p className="hint">{picked.length < 2 ? t('shell.chat.groupNeed') : picked.map((person) => person.displayName).join(', ')}</p>
          <Button type="submit" disabled={picked.length < 2}>{t('shell.chat.start')}</Button>
        </div>
      ) : null}
    </form>
  )
}

function InviteDialog({
  workspaceId,
  token,
  teams,
  onSubmit,
}: {
  workspaceId: string
  token: string | null
  teams: Team[]
  onSubmit: (email: string, role: string, teamId: string | null) => Promise<boolean>
}) {
  const { t } = useI18n()
  const [email, setEmail] = useState('')
  const [role, setRole] = useState('MEMBER')
  const [teamId, setTeamId] = useState('')
  const picked = teams.find((team) => team.id === teamId)
  const teamProjects = useQuery({ ...queries.teamProjects(workspaceId, picked?.id ?? ''), enabled: Boolean(picked) })
  const withRole = (teamProjects.data ?? []).filter((project) => project.teamRole !== null)
  const [sent, setSent] = useState(false)
  const workspaces = useQuery(queries.workspaces())
  const isOwner = workspaces.data?.find((workspace) => workspace.id === workspaceId)?.role === 'OWNER'
  return (
    <form
      onSubmit={(event: FormEvent) => {
        event.preventDefault()
        setSent(false)
        // The address stays in the box when the invitation failed, so it can be corrected.
        void onSubmit(email.trim(), role, picked?.id ?? null).then((ok) => {
          if (!ok) return
          setEmail('')
          setTeamId('')
          setSent(true)
        })
      }}
    >
      <Field label={t('invite.email')}>
        <input type="email" required value={email} onChange={(event) => { setEmail(event.target.value); setSent(false) }} />
      </Field>
      <Field label={t('invite.role')} hint={isOwner ? undefined : t('admin.invite.roleHint')}>
        <select value={role} onChange={(event) => setRole(event.target.value)}>
          <option value="MEMBER">{t('role.member')}</option>
          {isOwner ? <option value="ADMIN">{t('role.admin')}</option> : null}
          {isOwner ? <option value="OWNER">{t('admin.role.OWNER')}</option> : null}
        </select>
      </Field>
      <Field label={t('invite.team')} hint={t('invite.teamHint')}>
        <select value={picked?.id ?? ''} onChange={(event) => setTeamId(event.target.value)}>
          <option value="">{t('invite.teamNone')}</option>
          {teams.map((team) => <option key={team.id} value={team.id}>{team.name}</option>)}
        </select>
      </Field>
      {picked?.description ? <p className="hint team-description">{picked.description}</p> : null}
      {picked ? (
        <div className="invite-team-projects" role="status">
          {teamProjects.isPending ? <p className="hint">{t('loading')}</p> : null}
          {teamProjects.data && withRole.length > 0 ? (
            <>
              <strong>{t('invite.teamProjects', { count: String(withRole.length) })}</strong>
              <p className="hint">{t('invite.teamProjectsBody', { team: picked.name, projects: withRole.map((project) => `${project.name} (${t(`permissions.role.${project.teamRole!}`)})`).join(', ') })}</p>
            </>
          ) : null}
          {teamProjects.data && withRole.length === 0 ? <p className="hint">{t('invite.teamProjectsNone', { team: picked.name })}</p> : null}
        </div>
      ) : null}
      <Button type="submit">{t('create.invite')}</Button>
      {sent ? <p role="status">{token ? t('admin.invite.sent') : t('admin.invite.sentNoLink')}</p> : null}
      {token ? (
        <p className="invite-token">
          {t('invite.token')}: <Link to={`/invite/${token}`}>{token}</Link>
        </p>
      ) : null}
    </form>
  )
}
