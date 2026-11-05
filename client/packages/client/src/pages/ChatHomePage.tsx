import { useQuery } from '@tanstack/react-query'
import { Link, useParams } from 'react-router'
import { EmptyState } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { queries } from '../queries'
import { OpenCreate } from '../shell/WorkspaceShell'
import { channelScopeLabel, visibilityLabel } from '../modules/spaces/visibilityLabel'
import { canCreateChannel } from '../modules/spaces/capabilities'
import { useWorkspaceRole } from '../modules/spaces/useWorkspaceRole'

export function ChatHomePage() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const role = useWorkspaceRole(workspaceId)
  const channels = (space.data?.channels ?? []).filter((channel) => !channel.projectId)
  const projectChannels = (space.data?.channels ?? []).filter((channel) => channel.projectId)
  const channelSubtitle = (channel: { projectId: string | null; teamId: string | null; visibility: 'PUBLIC' | 'PRIVATE' }) => {
    const project = projectName(channel.projectId) || undefined
    const team = space.data?.teams.find((item) => item.id === channel.teamId)?.name
    const scope = channelScopeLabel(t, channel.visibility, project, team)
    return (channel.visibility === 'PUBLIC' && project && team ? [scope] : [project, scope]).filter(Boolean).join(' · ')
  }
  const projectName = (id: string | null) => space.data?.projects.find((project) => project.id === id)?.name ?? ''
  const dms = (space.data?.dms ?? []).filter((dm) => dm.lastMessageSeq == null || dm.lastMessageSeq > 0)
  return (
    <div className="page-scroll">
      <header className="page-intro">
        <h1>{t('nav.chat')}</h1>
        <p>{t('chat.landingBody')}</p>
      </header>
      {channels.length === 0 && projectChannels.length === 0 && dms.length === 0 ? (
        <EmptyState title={t('chat.emptyTitle')} body={t('chat.emptyBody')}>
          {canCreateChannel(role) ? <OpenCreate kind="channel" label={t('chat.createChannel')} /> : null}
          <OpenCreate kind="dm" label={t('chat.startDm')} />
        </EmptyState>
      ) : (
        <div className="result-list">
          {channels.map((channel) => (
            <Link key={channel.id} className="result-hit" to={`/w/${workspaceId}/c/${channel.conversationId}`}>
              <strong># {channel.name}</strong>
              <small>{visibilityLabel(t, channel.visibility, false)}</small>
            </Link>
          ))}
          {projectChannels.map((channel) => (
            <Link key={channel.id} className="result-hit" to={`/w/${workspaceId}/c/${channel.conversationId}`}>
              <strong># {channel.name}</strong>
              <small>{channelSubtitle(channel)}</small>
            </Link>
          ))}
          {dms.map((dm) => (
            <Link key={dm.id} className="result-hit" to={`/w/${workspaceId}/c/${dm.conversationId}`}>
              <strong>{dm.peers.map((peer) => peer.displayName).join(', ') || t('dm.short')}</strong>
              <small>{dm.lastMessagePreview ?? t('dm.short')}</small>
            </Link>
          ))}
        </div>
      )}
    </div>
  )
}
