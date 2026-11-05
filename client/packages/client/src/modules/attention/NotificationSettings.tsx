import { useEffect, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { DndSettings, NotificationPreferences } from '@penfii/contracts'
import { isVersionConflict } from '@penfii/api-client'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { localTimeZone } from './reminderTimes'

const DEFAULT_SCHEDULE = { days: [0, 1, 2, 3, 4], start: '22:00', end: '07:00' }

/**
 * What reaches the Inbox, Do Not Disturb and muted channels. Mentions, replies, assignments and
 * reminders always arrive; Do Not Disturb never removes anything from the Inbox.
 */
export function NotificationSettings() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const prefs = useQuery(queries.notificationPreferences())
  const muted = useQuery(queries.mutedChannels())
  const [level, setLevel] = useState<NotificationPreferences['level']>('ALL')
  const [dnd, setDnd] = useState<DndSettings>({ enabled: false, schedule: null, snoozeUntil: null })
  const [message, setMessage] = useState('')
  useEffect(() => {
    if (!prefs.data) return
    setLevel(prefs.data.level)
    setDnd(prefs.data.dnd)
  }, [prefs.data])

  if (prefs.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (prefs.isError || !prefs.data) return <p className="board-error" role="alert">{t('notify.error')}</p>
  const current = prefs.data
  const schedule = dnd.schedule

  async function save(next: DndSettings = dnd) {
    setMessage('')
    try {
      await api.saveNotificationPreferences({ level, dnd: next, external: current.external, expectedVersion: current.version })
      setMessage(t('notify.saved'))
    } catch (error) {
      setMessage(isVersionConflict(error) ? t('notify.conflict') : t('notify.error'))
    }
    await queryClient.invalidateQueries({ queryKey: queryKeys.notificationPreferences })
  }
  function toggleDay(day: number) {
    if (!schedule) return
    const days = schedule.days.includes(day) ? schedule.days.filter((d) => d !== day) : [...schedule.days, day].sort()
    setDnd({ ...dnd, schedule: { ...schedule, days } })
  }

  return (
    <section className="card settings-card notification-settings" aria-labelledby="notify-title">
      <h2 id="notify-title">{t('notify.title')}</h2>
      <p>{t('notify.lead')}</p>
      <fieldset>
        <legend>{t('notify.level')}</legend>
        {(['ALL', 'DIRECT_ONLY'] as const).map((value) => (
          <label key={value} className="radio-row">
            <input type="radio" name="notify-level" checked={level === value} onChange={() => setLevel(value)} />
            {t(`notify.level.${value}`)}
          </label>
        ))}
      </fieldset>
      <fieldset>
        <legend>{t('notify.dnd')}</legend>
        <p className="hint">{t('notify.dnd.help')}</p>
        <label className="radio-row">
          <input
            type="checkbox"
            checked={dnd.enabled}
            onChange={(event) => setDnd({ ...dnd, enabled: event.target.checked, schedule: event.target.checked && !dnd.schedule ? { ...DEFAULT_SCHEDULE, timeZone: localTimeZone() } : dnd.schedule })}
          />
          {t('notify.dnd.enabled')}
        </label>
        {dnd.enabled && schedule ? (
          <div className="dnd-schedule">
            <div role="group" aria-label={t('notify.dnd.days')}>
              {([0, 1, 2, 3, 4, 5, 6] as const).map((day) => (
                <label key={day} className="day-toggle">
                  <input type="checkbox" checked={schedule.days.includes(day)} onChange={() => toggleDay(day)} />
                  {t(`notify.day.${day}`)}
                </label>
              ))}
            </div>
            <label>{t('notify.dnd.start')} <input type="time" value={schedule.start} onChange={(event) => setDnd({ ...dnd, schedule: { ...schedule, start: event.target.value } })} /></label>
            <label>{t('notify.dnd.end')} <input type="time" value={schedule.end} onChange={(event) => setDnd({ ...dnd, schedule: { ...schedule, end: event.target.value } })} /></label>
            <small className="hint">{schedule.timeZone}</small>
          </div>
        ) : null}
        <Button type="button" variant="secondary" onClick={() => void save({ ...dnd, enabled: true, snoozeUntil: new Date(Date.now() + 3_600_000).toISOString() })}>{t('notify.dnd.snooze')}</Button>
        {current.dndActive ? <p className="hint" role="status">{t('notify.dnd.active')}</p> : null}
      </fieldset>
      <Button type="button" onClick={() => void save()}>{t('notify.save')}</Button>
      {message ? <p role="status">{message}</p> : null}
      <MutedChannels ids={muted.data?.channelIds ?? []} />
    </section>
  )
}

function MutedChannels({ ids }: { ids: string[] }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  return (
    <div className="muted-channels">
      <h3>{t('notify.muted.title')}</h3>
      {ids.length === 0 ? <p className="hint">{t('notify.muted.empty')}</p> : null}
      <ul>
        {ids.map((id) => (
          <li key={id}>
            <ChannelName id={id} />{' '}
            <Button type="button" variant="ghost" onClick={() => void api.muteChannel(id, false).then(() => queryClient.invalidateQueries({ queryKey: queryKeys.mutedChannels }))}>{t('notify.muted.unmute')}</Button>
          </li>
        ))}
      </ul>
    </div>
  )
}

function ChannelName({ id }: { id: string }) {
  const queryClient = useQueryClient()
  // The Space list is already loaded for the shell; read the channel's name from it when present.
  const spaces = queryClient.getQueriesData<{ channels: Array<{ id: string; name: string }> }>({ queryKey: queryKeys.all.space })
  for (const [, space] of spaces) {
    const channel = space?.channels.find((item) => item.id === id)
    if (channel) return <strong>#{channel.name}</strong>
  }
  return <strong>#{id.slice(0, 8)}</strong>
}
