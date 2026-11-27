import { MuteButton } from '../modules/attention/MuteButton'
import { useEffect, useRef, useState } from 'react'
import type { Channel } from '@penfii/contracts'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useParams, useSearchParams } from 'react-router'
import { createPortal } from 'react-dom'
import { api } from '../api'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { NotFoundState } from '../ui/NotFoundState'
import { QueryError } from '../ui/QueryError'
import { ConversationUnavailable } from '../modules/conversations/ConversationUnavailable'
import { ConversationPane } from './ConversationPane'
import { MemberStack } from '../modules/people/MemberStack'
import { channelScopeLabel, visibilityLabel } from '../modules/spaces/visibilityLabel'
import { canChangeChannel, canChangeVisibility, canInviteToChannel, canLeaveChannel, canRunChannelPeople, canSetChannelRoles } from '../modules/spaces/capabilities'
import { ChannelMembersDialog } from '../modules/spaces/ChannelMembersDialog'
import { LeaveChannelButton } from '../modules/spaces/LeaveChannelButton'
import { Button, useConfirm } from '../design/ui'
import { ChannelWebhooksButton } from '../modules/conversations/ChannelWebhooks'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'

export function ConversationFrame() {
  const { t } = useI18n()
  const params = useParams()
  const [searchParams, setSearchParams] = useSearchParams()
  const workspaceId = params.workspaceId
  const conversationId = params.conversationId
  const rootId = params.rootId
  const session = useSession()
  const workspaceRole = useWorkspaceRole(workspaceId ?? '')
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [peopleOpen, setPeopleOpen] = useState(false)
  const space = useQuery({ ...queries.space(workspaceId ?? ''), enabled: Boolean(workspaceId) })

  const listed = space.data?.channels.find((item) => item.conversationId === conversationId)
  const memberProjectId = listed?.projectId
  // Everyone in the Project reads its public channels; a private one is read by its own members only.
  const privateChannelId = memberProjectId && listed?.visibility === 'PRIVATE' ? listed.id : undefined
  const projectMembers = useQuery({ ...queries.projectMembers(memberProjectId ?? ''), enabled: Boolean(memberProjectId) && !privateChannelId })
  const channelMembers = useQuery({ ...queries.channelMembers(privateChannelId ?? ''), enabled: Boolean(privateChannelId) })
  const members = privateChannelId ? channelMembers.data : projectMembers.data

  // A conversation missing from the list is called unavailable only after the list was read again for it: a list read before
  // the person was put into the channel (or before a conversation was made) must not decide.
  const dmListed = space.data?.dms.some((item) => item.conversationId === conversationId)
  const missing = space.isSuccess && !listed && !dmListed
  const [confirmedMissing, setConfirmedMissing] = useState<string | null>(null)
  useEffect(() => {
    if (!missing || !workspaceId || !conversationId || confirmedMissing === conversationId) return
    let cancelled = false
    void queryClient
      .fetchQuery({ ...queries.space(workspaceId), staleTime: 0 })
      .then((fresh) => {
        const there = fresh.channels.some((item) => item.conversationId === conversationId) || fresh.dms.some((item) => item.conversationId === conversationId)
        if (!cancelled && !there) setConfirmedMissing(conversationId)
      })
      // A list that cannot be read is the list's own error (shown below, with a way to try again), not a missing conversation.
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [missing, workspaceId, conversationId, confirmedMissing, queryClient])

  if (!workspaceId || !conversationId) return <NotFoundState />
  // Who is signed in is not known yet (or could not be read): not a conversation that is missing.
  if (!session.data) return session.isError ? <div className="page-scroll"><QueryError query={session} /></div> : <p className="hint" role="status">{t('loading')}</p>
  const channel = space.data?.channels.find((item) => item.conversationId === conversationId)
  const dm = space.data?.dms.find((item) => item.conversationId === conversationId)
  const team = space.data?.teams.find((item) => item.id === channel?.teamId)
  const project = space.data?.projects.find((item) => item.id === channel?.projectId)
  const peerName = dm?.peers.map((peer) => peer.displayName).join(', ')
  // A conversation the person cannot read is not in their Space: say so, and where to go, instead of a page stuck on "loading".
  if (!channel && !dm && space.isError) return <div className="page-scroll"><QueryError query={space} /></div>
  if (missing && confirmedMissing === conversationId) return <ConversationUnavailable workspaceId={workspaceId} projectId={params.projectId} />
  if (missing) return <p className="hint" role="status">{t('loading')}</p>
  const title = channel ? `# ${channel.name}` : peerName || (dm ? t('dm.short') : t('loading'))
  const scopeText = channel ? channelScopeLabel(t, channel.visibility, project?.name, team?.name) : null
  const namesProject = channel?.visibility === 'PUBLIC' && team && project
  const subtitle = [namesProject ? null : project?.name, scopeText, members ? t('conversation.memberCount', { count: String(members.length) }) : null]
    .filter((item): item is string => Boolean(item))
    .join(' · ')
  // Threads stay in the part of the app the conversation was opened from (Chat or its Project).
  const threadBase = params.projectId
    ? `/w/${workspaceId}/projects/${params.projectId}/chat/${conversationId}`
    : `/w/${workspaceId}/c/${conversationId}`
  const names: Record<string, string> = { [session.data.id]: session.data.displayName }
  for (const peer of dm?.peers ?? []) names[peer.id] = peer.displayName

  return (
    <div className="conversation-frame">
      <header className="context-bar">
        <div>
          <h1>{title}</h1>
          {subtitle ? <p>{subtitle}</p> : null}
        </div>
        {channel ? (
          <div className="context-actions">
            <MemberStack members={members ?? []} />
            <MuteButton channelId={channel.id} />
            {/* A member who does not run the channel's people still sees them here, and invites when the channel allows it. */}
            {channel.visibility === 'PRIVATE' && !canRunChannelPeople(channel, project) ? (
              <ChannelPeopleButton workspaceId={workspaceId} channelId={channel.id} canAdd={canInviteToChannel(channel, project)} canSetRoles={false} canRemove={false} canSetInvites={false} allowMemberInvites={channel.allowMemberInvites} open={peopleOpen} onOpenChange={setPeopleOpen} />
            ) : null}
            {canLeaveChannel(channel) ? (
              <LeaveChannelButton
                channelId={channel.id}
                channelName={channel.name}
                userId={session.data.id}
                onChooseManager={() => {
                  setSearchParams((current) => { const next = new URLSearchParams(current); next.set('settings', '1'); next.set('members', '1'); return next })
                }}
                onLeft={() => {
                  // Go first: the channel is no longer in the list, and a page for it would say "not found".
                  void navigate(params.projectId ? `/w/${workspaceId}/projects/${params.projectId}/chat` : `/w/${workspaceId}/chat`)
                  void queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
                }}
              />
            ) : null}
          </div>
        ) : null}
      </header>
      <ConversationPane
        workspaceId={workspaceId}
        conversationId={conversationId}
        threadRootId={rootId ?? null}
        currentUserId={session.data.id}
        names={names}
        showDmWarning={false}
        heading={title}
        threadBase={threadBase}
        projectId={params.projectId}
      />
      {channel && searchParams.get('settings') === '1' ? createPortal(
        <ChannelSettingsPanel
          workspaceId={workspaceId}
          channel={channel}
          project={project}
          team={team}
          workspaceRole={workspaceRole}
          peopleOpen={searchParams.get('members') === '1'}
          onPeopleOpenChange={(open) => setSearchParams((current) => { const next = new URLSearchParams(current); if (open) next.set('members', '1'); else next.delete('members'); return next })}
          onChanged={() => void queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })}
          onClose={() => setSearchParams((current) => { const next = new URLSearchParams(current); next.delete('settings'); next.delete('members'); return next })}
        />, document.body,
      ) : null}
    </div>
  )
}

function ChannelSettingsPanel({ workspaceId, channel, project, team, workspaceRole, peopleOpen, onPeopleOpenChange, onChanged, onClose }: {
  workspaceId: string
  channel: Channel
  project: { id: string; name: string; canManage?: boolean } | undefined
  team: { id: string; name: string } | undefined
  workspaceRole: string | undefined
  peopleOpen: boolean
  onPeopleOpenChange: (open: boolean) => void
  onChanged: () => void
  onClose: () => void
}) {
  const { t } = useI18n()
  const panel = useRef<HTMLElement>(null)
  const closeRef = useRef(onClose)
  closeRef.current = onClose
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    panel.current?.focus()
    const onKey = (event: KeyboardEvent) => {
      if (event.target instanceof Element && event.target.closest('dialog[open]')) return
      if (event.key === 'Escape') { event.stopPropagation(); closeRef.current(); return }
      if (event.key === 'Tab') {
        const focusable = [...(panel.current?.querySelectorAll<HTMLElement>('button:not([disabled]),a[href],input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])') ?? [])]
        if (!focusable.length) { event.preventDefault(); panel.current?.focus(); return }
        const first = focusable[0]
        const last = focusable[focusable.length - 1]
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
      }
    }
    document.addEventListener('keydown', onKey, true)
    return () => { document.removeEventListener('keydown', onKey, true); if (previous?.isConnected) previous.focus() }
  }, [])
  const canManage = canChangeChannel(channel, workspaceRole, project)
  const canVisibility = canChangeVisibility(channel, workspaceRole, project)
  const canPeople = channel.visibility === 'PRIVATE' && canRunChannelPeople(channel, project)
  return (
    <div className="channel-settings-layer">
      <button className="channel-settings-backdrop" type="button" aria-label={t('close')} onClick={onClose} />
      <aside ref={panel} className="channel-settings-panel" role="dialog" aria-modal="true" aria-label={t('channel.settings')} tabIndex={-1}>
        <header className="channel-settings-head"><div><h2>{t('channel.settings')}</h2><p># {channel.name}</p></div><Button type="button" variant="secondary" onClick={onClose}>{t('close')}</Button></header>
        <div className="channel-settings-body">
          {canManage ? <section><h3>{t('channel.webhooks')}</h3><ChannelWebhooksButton channelId={channel.id} /></section> : null}
          {canVisibility ? <section><h3>{t('channel.visibility')}</h3><VisibilitySelect channelId={channel.id} visibility={channel.visibility} inProject={Boolean(channel.projectId)} publicLabel={channelScopeLabel(t, 'PUBLIC', project?.name, team?.name)} onChanged={onChanged} /></section> : null}
          {canPeople ? <section><h3>{t('channel.addPeople')}</h3><ChannelPeopleButton workspaceId={workspaceId} channelId={channel.id} canAdd={canInviteToChannel(channel, project)} canSetRoles={canSetChannelRoles(channel, project)} canRemove={canRunChannelPeople(channel, project)} canSetInvites={canRunChannelPeople(channel, project)} allowMemberInvites={channel.allowMemberInvites} open={peopleOpen} onOpenChange={onPeopleOpenChange} /></section> : null}
          {!canManage && !canVisibility && !canPeople ? <p className="hint">{t('channel.settingsReadOnly')}</p> : null}
        </div>
      </aside>
    </div>
  )
}


function ChannelPeopleButton({
  workspaceId,
  channelId,
  canAdd,
  canSetRoles,
  canRemove,
  canSetInvites,
  allowMemberInvites,
  open,
  onOpenChange,
}: {
  workspaceId: string
  channelId: string
  canAdd: boolean
  canSetRoles: boolean
  canRemove: boolean
  canSetInvites: boolean
  allowMemberInvites?: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useI18n()
  const setOpen = onOpenChange
  return (
    <>
      <Button type="button" onClick={() => setOpen(true)}>
        {t('channel.addPeople')}
      </Button>
      <ChannelMembersDialog open={open} onClose={() => setOpen(false)} workspaceId={workspaceId} channelId={channelId} canAdd={canAdd} canSetRoles={canSetRoles} canRemove={canRemove} canSetInvites={canSetInvites} allowMemberInvites={allowMemberInvites} />
    </>
  )
}

function VisibilitySelect({
  channelId,
  visibility,
  inProject,
  publicLabel,
  onChanged,
}: {
  channelId: string
  visibility: 'PUBLIC' | 'PRIVATE'
  inProject: boolean
  publicLabel: string
  onChanged: () => void
}) {
  const { t } = useI18n()
  const { confirm, dialog } = useConfirm()
  const [value, setValue] = useState(visibility)
  const [error, setError] = useState(false)
  // Turning it private closes it to everyone who is not a member; turning it public shows what was said
  // in it to people who could not read it. Neither changes who is a member. Say so before doing it.
  async function change(next: 'PUBLIC' | 'PRIVATE') {
    if (next === value) return
    const body = next === 'PRIVATE' ? t('channel.visibility.confirmPrivate') : t('channel.visibility.confirmPublic')
    if (!(await confirm({ title: t('channel.visibility'), body, confirmLabel: t('channel.visibility.change'), tone: next === 'PUBLIC' ? 'danger' : 'primary' }))) return
    setValue(next)
    setError(false)
    void api.updateChannel(channelId, { visibility: next }).then(onChanged).catch(() => {
      setValue(visibility)
      setError(true)
    })
  }
  return (
    <>
      {dialog}
      <label className="hint">
        {t('channel.visibility')}
        <select
          value={value}
          onChange={(event) => void change(event.target.value === 'PRIVATE' ? 'PRIVATE' : 'PUBLIC')}
        >
          <option value="PUBLIC">{publicLabel}</option>
          <option value="PRIVATE">{visibilityLabel(t, 'PRIVATE', inProject)}</option>
        </select>
        {error ? <span className="form-error">{t('error.generic')}</span> : null}
      </label>
    </>
  )
}
