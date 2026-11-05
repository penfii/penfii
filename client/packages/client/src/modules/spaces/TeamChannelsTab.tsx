import { Link } from 'react-router'
import type { Channel } from '@penfii/contracts'
import { Badge } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'

/** The channels that belong to a Team. They are read-only while the Team is archived. */
export function TeamChannelsTab({ workspaceId, channels, archived }: { workspaceId: string; channels: Channel[]; archived: boolean }) {
  const { t } = useI18n()
  return (
    <section className="team-channels" aria-label={t('teamPage.tab.channels')}>
      {archived ? <p className="hint">{t('teams.channelReadOnly')}</p> : null}
      {channels.length === 0 ? <p className="hint">{t('teamPage.channels.empty')}</p> : (
        <ul className="plain-list">
          {channels.map((channel) => (
            <li key={channel.id} className="team-channel-row">
              <Link to={`/w/${workspaceId}/c/${channel.conversationId}`}><strong># {channel.name}</strong></Link>
              {channel.visibility === 'PRIVATE' ? <Badge>{t('teamPage.channels.private')}</Badge> : null}
              {archived ? <Badge>{t('teamPage.channels.readOnly')}</Badge> : null}
              {channel.description ? <small className="hint">{channel.description}</small> : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
