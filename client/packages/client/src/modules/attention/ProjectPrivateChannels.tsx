import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { ManagedChannel } from '@penfii/contracts'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { queryKeys } from '../../queryKeys'
import { ChannelWebhooksButton } from '../conversations/ChannelWebhooks'
import { ChannelMembersDialog } from '../spaces/ChannelMembersDialog'

/**
 * The private channels of a Project for its admin, read or not. The admin runs the people of each (who is
 * in, who manages, whether members may invite) and sees and revokes its webhooks, but what is said in a
 * channel they are not in stays closed to them: only names and counts are listed here, and an admin who
 * is not in a channel cannot put themself in. A channel the admin is in is listed too, so all of them
 * are in one place.
 */
export function ProjectPrivateChannels({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { t } = useI18n()
  const channels = useQuery(queries.projectPrivateChannels(projectId))
  if (!channels.data?.length) return null
  return (
    <section className="permissions" aria-labelledby="private-channels-title">
      <h3 id="private-channels-title">{t('projectChannels.title')}</h3>
      <p className="hint">{t('projectChannels.hint')}</p>
      <ul className="plain-list">
        {channels.data.map((channel) => (
          <ChannelRow key={channel.id} workspaceId={workspaceId} projectId={projectId} channel={channel} />
        ))}
      </ul>
    </section>
  )
}

function ChannelRow({ workspaceId, projectId, channel }: { workspaceId: string; projectId: string; channel: ManagedChannel }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  return (
    <li className="admin-no-admin-row">
      <strong>{`# ${channel.name}`}</strong>{' '}
      <small className="hint">
        {t('projectChannels.counts', { members: String(channel.memberCount), managers: String(channel.managerCount) })}
        {channel.isMember ? ` · ${t('projectChannels.inIt')}` : ` · ${t('projectChannels.notInIt')}`}
        {channel.archived ? ` · ${t('projectChannels.archived')}` : ''}
      </small>
      {channel.description ? <p className="hint">{channel.description}</p> : null}
      <div className="row-actions">
        <Button type="button" variant="secondary" aria-label={`${t('projectChannels.people')}: ${channel.name}`} onClick={() => setOpen(true)}>
          {t('projectChannels.people')}
        </Button>
        {/* Seeing and revoking is for the admin; a posting credential is for someone who may post. */}
        <ChannelWebhooksButton channelId={channel.id} canIssue={channel.isMember} />
      </div>
      <ChannelMembersDialog
        open={open}
        onClose={() => setOpen(false)}
        workspaceId={workspaceId}
        channelId={channel.id}
        canAdd
        canSetRoles
        canRemove
        canSetInvites
        viewerIsMember={channel.isMember}
        allowMemberInvites={channel.allowMemberInvites}
        onChanged={() => void queryClient.invalidateQueries({ queryKey: queryKeys.projectPrivateChannels(projectId) })}
      />
    </li>
  )
}
