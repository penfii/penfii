import { Button } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { formatStamp } from '../../format'
import { usePushSubscription } from './usePushSubscription'

/** A failure counts while no later push was accepted (the server keeps the last of each). */
function failureSince(device: { lastSuccessAt: string | null; lastFailureAt: string | null }): string | null {
  const failed = device.lastFailureAt
  if (!failed) return null
  return !device.lastSuccessAt || Date.parse(failed) > Date.parse(device.lastSuccessAt) ? failed : null
}

/**
 * Web Push on this device: one switch (no per-kind settings), the state it is in, and the person's
 * other devices. Hidden while the server has no VAPID key. Which Inbox items are pushed is the
 * server's rule (DMs, mentions, replies, assignments, reminders). Each device shows when the push service last accepted a push for it (not that the device showed it) and a failure since then.
 */
export function PushSettings() {
  const { t, locale } = useI18n()
  const push = usePushSubscription()
  const { state } = push
  if (state.kind === 'unconfigured') return null
  if (state.kind === 'loading') return null
  const on = state.kind === 'on'
  return (
    <section className="card settings-card push-settings" aria-labelledby="push-title">
      <h2 id="push-title">{t('push.title')}</h2>
      <p>{t('push.lead')}</p>
      {state.kind === 'ios-needs-install' ? (
        <div className="push-note" role="note">
          <p><strong>{t('push.ios.title')}</strong></p>
          <ol>
            <li>{t('push.ios.step1')}</li>
            <li>{t('push.ios.step2')}</li>
            <li>{t('push.ios.step3')}</li>
          </ol>
          <p className="hint">{t('push.ios.minVersion')}</p>
        </div>
      ) : null}
      {state.kind === 'unsupported' ? <p className="hint" role="status">{t('push.unsupported')}</p> : null}
      {state.kind === 'denied' ? <p className="board-note" role="status">{t('push.denied')}</p> : null}
      {state.kind === 'on' || state.kind === 'off' ? (
        <label className="radio-row">
          <input
            type="checkbox"
            role="switch"
            checked={on}
            disabled={push.busy}
            onChange={(event) => void (event.target.checked ? push.turnOn() : push.turnOff())}
          />
          {t('push.thisDevice')}
        </label>
      ) : null}
      {push.failed ? <p className="board-error" role="alert">{t('push.error')}</p> : null}
      <p className="hint">{t('push.kinds')}</p>
      {push.devices.length > 0 ? (
        <>
          <h3>{t('push.devices')}</h3>
          <ul className="result-list push-devices">
            {push.devices.map((device) => {
              const failure = failureSince(device)
              return (
                <li key={device.id} className="result-hit">
                  <span>
                    <strong>{device.label ?? t('push.device')}</strong>
                    {device.endpoint === push.thisEndpoint ? ` · ${t('push.thisOne')}` : ''}
                    <small className="hint"> {t('push.since', { date: formatStamp(device.createdAt, locale) })}</small>
                    <small className="hint push-delivery">
                      {device.lastSuccessAt ? ` · ${t('push.lastOk', { date: formatStamp(device.lastSuccessAt, locale) })}` : ` · ${t('push.noneYet')}`}
                      {failure ? ` · ${t('push.lastFailed', { date: formatStamp(failure, locale) })}` : ''}
                    </small>
                  </span>
                  <Button type="button" variant="secondary" disabled={push.busy} aria-label={t('push.remove', { name: device.label ?? t('push.device') })} onClick={() => void push.removeDevice(device)}>
                    {t('push.removeShort')}
                  </Button>
                </li>
              )
            })}
          </ul>
        </>
      ) : null}
    </section>
  )
}
