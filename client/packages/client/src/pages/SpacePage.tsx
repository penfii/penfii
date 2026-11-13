import { MuteButton } from '../modules/attention/MuteButton'
import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useParams } from 'react-router'
import { api } from '../api'
import { useI18n } from '../i18n/I18nProvider'
import { useSession } from '../modules/identity'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { NotFoundState } from '../ui/NotFoundState'
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
  const workspaceId = params.workspaceId
  const conversationId = params.conversationId
  const rootId = params.rootId
  const session = useSession()
  const workspaceRole = useWorkspaceRole(workspaceId ?? '')
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [peopleOpen, setPeopleOpen] = useState(false)
  const space = useQuery({ ...queries.space(workspaceId ?? ''), enabled: Boolean(workspaceId) })

  const memberProjectId = space.data?.channels.find((item) => item.conversationId === conversationId)?.projectId
  const projectMembers = useQuery({ ...queries.projectMembers(memberProjectId ?? ''), enabled: Boolean(memberProjectId) })

  if (!workspaceId || !conversationId || !session.data) return <NotFoundState />
  const channel = space.data?.channels.find((item) => item.conversationId === conversationId)
  const dm = space.data?.dms.find((item) => item.conversationId === conversationId)
  const team = space.data?.teams.find((item) => item.id === channel?.teamId)
  const project = space.data?.projects.find((item) => item.id === channel?.projectId)
  const peerName = dm?.peers.map((peer) => peer.displayName).join(', ')
  // A conversation the person cannot read is not in their Space: say so instead of a page stuck on "loading".
  if (space.isSuccess && !channel && !dm) return <NotFoundState />
  const title = channel ? `# ${channel.name}` : peerName || (dm ? t('dm.short') : t('loading'))
  const scopeText = channel ? channelScopeLabel(t, channel.visibility, project?.name, team?.name) : null
  const namesProject = channel?.visibility === 'PUBLIC' && team && project
  const subtitle = [namesProject ? null : project?.name, scopeText, projectMembers.data ? t('conversation.memberCount', { count: String(projectMembers.data.length) }) : null]
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
            <MemberStack members={projectMembers.data ?? []} />
            <MuteButton channelId={channel.id} />
            {channel.visibility === 'PRIVATE' ? (
              <ChannelPeopleButton workspaceId={workspaceId} channelId={channel.id} canAdd={canInviteToChannel(channel, project)} canSetRoles={canSetChannelRoles(channel, project)} canRemove={canRunChannelPeople(channel, project)} canSetInvites={canRunChannelPeople(channel, project)} allowMemberInvites={channel.allowMemberInvites} open={peopleOpen} onOpenChange={setPeopleOpen} />
            ) : null}
            {canLeaveChannel(channel) ? (
              <LeaveChannelButton
                channelId={channel.id}
                channelName={channel.name}
                userId={session.data.id}
                onChooseManager={() => setPeopleOpen(true)}
                onLeft={() => {
                  // Go first: the channel is no longer in the list, and a page for it would say "not found".
                  void navigate(params.projectId ? `/w/${workspaceId}/projects/${params.projectId}/chat` : `/w/${workspaceId}/chat`)
                  void queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
                }}
              />
            ) : null}
            {canChangeChannel(channel, workspaceRole, project) ? (
              <ChannelWebhooksButton channelId={channel.id} />
            ) : null}
            {canChangeVisibility(channel, workspaceRole, project) ? (
              <VisibilitySelect
                channelId={channel.id}
                visibility={channel.visibility}
            inProject={Boolean(channel.projectId)}
            publicLabel={channelScopeLabel(t, 'PUBLIC', project?.name, team?.name)}
                onChanged={() => void queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })}
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
      />
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
