import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { LaterItem } from '@penfii/contracts'
import { api } from '../../api'
import { Button } from '../../design/ui'
import { formatStamp } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { localTimeZone, reminderPresets, toLocalInput } from './reminderTimes'
import { queryKeys } from '../../queryKeys'

/** A one-shot Reminder on a Later card. It stops by itself when the card is finished. */
export function ReminderControl({ item }: { item: LaterItem }) {
  const { t, locale } = useI18n()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [custom, setCustom] = useState('')
  const [error, setError] = useState('')
  const reminder = item.reminder ?? null
  const scheduled = reminder?.state === 'SCHEDULED'

  async function save(at: Date) {
    setError('')
    if (at.getTime() < Date.now() - 60_000) {
      setError(t('reminder.past'))
      return
    }
    try {
      await api.setReminder(item.id, { dueAt: at.toISOString(), timeZone: localTimeZone() })
      setOpen(false)
    } catch {
      setError(t('reminder.error'))
    }
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.later })
  }
  async function cancel() {
    await api.cancelReminder(item.id)
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.later })
  }

  if (item.state !== 'ACTIVE') {
    return reminder?.state === 'FIRED' && reminder.firedAt ? <small className="hint">{t('reminder.fired', { time: formatStamp(reminder.firedAt, locale) })}</small> : null
  }
  return (
    <div className="reminder-control">
      {scheduled && reminder ? <small className="hint">⏰ {t('reminder.scheduled', { time: formatStamp(reminder.dueAt, locale) })}</small> : null}
      {!scheduled && reminder?.state === 'FIRED' && reminder.firedAt ? <small className="hint">{t('reminder.fired', { time: formatStamp(reminder.firedAt, locale) })}</small> : null}
      <Button type="button" variant="ghost" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        {scheduled ? t('reminder.change') : t('reminder.set')}
      </Button>
      {scheduled ? <Button type="button" variant="ghost" onClick={() => void cancel()}>{t('reminder.cancel')}</Button> : null}
      {open ? (
        <div className="reminder-panel" role="group" aria-label={t('reminder.title')}>
          {reminderPresets().map((preset) => (
            <Button key={preset.id} type="button" variant="secondary" onClick={() => void save(preset.at)}>{t(`reminder.${preset.id}`)}</Button>
          ))}
          <label>
            {t('reminder.custom')}
            <input type="datetime-local" value={custom} min={toLocalInput(new Date())} onChange={(event) => setCustom(event.target.value)} />
          </label>
          <Button type="button" disabled={custom === ''} onClick={() => void save(new Date(custom))}>{t('reminder.save')}</Button>
          <small className="hint">{t('reminder.help')}</small>
          {error ? <p className="form-error" role="alert">{error}</p> : null}
        </div>
      ) : null}
    </div>
  )
}
