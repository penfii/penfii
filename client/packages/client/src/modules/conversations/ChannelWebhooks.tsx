import { useState, type FormEvent } from 'react'
import { queryKeys } from '../../queryKeys'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { Webhook, WebhookCreated } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { useReauth } from '../identity/reauth'
import { errorText } from '../../ui/errorText'

/** Incoming webhooks for one channel. The secret from create or rotate stays on screen until the person dismisses it. */
export function ChannelWebhooksButton({ channelId, canIssue = true }: { channelId: string; canIssue?: boolean }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button type="button" onClick={() => setOpen(true)}>{t('channel.webhooks')}</Button>
      {open ? <ChannelWebhooks channelId={channelId} canIssue={canIssue} onClose={() => setOpen(false)} /> : null}
    </>
  )
}

/** `canIssue` false: the caller runs the channel but may not post to it, so they only see the webhooks and revoke them. */
function ChannelWebhooks({ channelId, canIssue, onClose }: { channelId: string; canIssue: boolean; onClose: () => void }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const reauth = useReauth()
  const list = useQuery({
    queryKey: queryKeys.channelWebhooks(channelId),
    queryFn: () => api.listChannelWebhooks(channelId),
  })
  const [name, setName] = useState('')
  const [actor, setActor] = useState('')
  const [mode, setMode] = useState<'URL_SECRET' | 'SIGNATURE'>('URL_SECRET')
  const [secret, setSecret] = useState<WebhookCreated | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  function refresh() {
    return queryClient.invalidateQueries({ queryKey: queryKeys.channelWebhooks(channelId) })
  }

  async function act(work: () => Promise<void>) {
    setBusy(true)
    setError(null)
    try {
      await work()
    } catch (caught) {
      setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    } finally {
      setBusy(false)
    }
  }

  function create(event: FormEvent) {
    event.preventDefault()
    return act(async () => {
      const created = await reauth.run(() => api.createChannelWebhook(channelId, {
        name: name.trim(),
        authMode: mode,
        displayName: actor.trim(),
      }))
      if (!created) return
      setSecret(created)
      setName('')
      setActor('')
      await refresh()
    })
  }

  function rotate(webhook: Webhook) {
    return act(async () => {
      const created = await reauth.run(() => api.rotateWebhook(webhook.id))
      if (!created) return
      setSecret(created)
      await refresh()
    })
  }

  function revoke(webhook: Webhook) {
    return act(async () => {
      const revoked = await reauth.run(() => api.revokeWebhook(webhook.id))
      if (!revoked) return
      setSecret(null)
      await refresh()
    })
  }

  return (
    <Dialog open title={t('channel.webhooks.title')} onClose={onClose}>
      <div>
        {secret ? (
          <div>
            <p>{t('channel.webhooks.secretOnce')}</p>
            <p>{secret.webhook.authMode === 'SIGNATURE' ? t('channel.webhooks.signatureHint') : t('channel.webhooks.urlHint')}</p>
            <p><code>{secret.secret}</code></p>
            <div className="dialog-actions">
              <Button type="button" onClick={() => setSecret(null)}>{t('channel.webhooks.done')}</Button>
            </div>
          </div>
        ) : (
          <>
            {list.data && list.data.items.length === 0 ? <p className="hint">{t('channel.webhooks.empty')}</p> : null}
            <ul className="plain-list">
              {(list.data?.items ?? []).map((webhook) => (
                <li key={webhook.id}>
                  <strong>{webhook.name}</strong>
                  <span className="hint"> {webhook.actorDisplayName} · {webhook.revoked ? t('channel.webhooks.revoked') : t('channel.webhooks.active')}</span>
                  {webhook.revoked ? null : (
                    <span className="dialog-actions">
                      {canIssue ? <Button type="button" variant="secondary" disabled={busy} onClick={() => void rotate(webhook)}>{t('channel.webhooks.rotate')}</Button> : null}
                      <Button type="button" variant="secondary" disabled={busy} onClick={() => void revoke(webhook)}>{t('channel.webhooks.revoke')}</Button>
                    </span>
                  )}
                </li>
              ))}
            </ul>
            {canIssue ? (
            <form onSubmit={(event) => void create(event)}>
              <label>
                {t('channel.webhooks.name')}
                <input value={name} required onChange={(event) => setName(event.target.value)} />
              </label>
              <label>
                {t('channel.webhooks.actor')}
                <input value={actor} required onChange={(event) => setActor(event.target.value)} />
              </label>
              <label>
                {t('channel.webhooks.mode')}
                <select value={mode} onChange={(event) => setMode(event.target.value === 'SIGNATURE' ? 'SIGNATURE' : 'URL_SECRET')}>
                  <option value="URL_SECRET">{t('channel.webhooks.mode.url')}</option>
                  <option value="SIGNATURE">{t('channel.webhooks.mode.signature')}</option>
                </select>
              </label>
              {error ? <p className="form-error">{error}</p> : null}
              <div className="dialog-actions">
                <Button type="button" variant="secondary" onClick={onClose}>{t('channel.webhooks.done')}</Button>
                <Button type="submit" disabled={busy}>{t('channel.webhooks.create')}</Button>
              </div>
            </form>
            ) : (
              <>
                <p className="hint">{t('channel.webhooks.revokeOnly')}</p>
                {error ? <p className="form-error">{error}</p> : null}
                <div className="dialog-actions">
                  <Button type="button" variant="secondary" onClick={onClose}>{t('channel.webhooks.done')}</Button>
                </div>
              </>
            )}
          </>
        )}
        {reauth.dialog}
      </div>
    </Dialog>
  )
}
