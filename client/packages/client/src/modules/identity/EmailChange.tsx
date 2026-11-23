import { useState, type FormEvent } from 'react'
import { isApiError } from '@penfii/api-client'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Button, Field } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { queryKeys } from '../../queryKeys'
import { useReauth } from './reauth'
import { useSession } from './useSession'

/**
 * Changing the e-mail address: a link goes to the new address and the address changes when it is
 * opened (the account keeps signing in with the current one until then). Without server e-mail the
 * change is not offered; the operator does it.
 */
export function EmailChange() {
  const { t } = useI18n()
  const session = useSession()
  const queryClient = useQueryClient()
  const info = useQuery({ ...queries.serverInfo(), retry: false })
  const pending = useQuery({ queryKey: queryKeys.emailChange, queryFn: () => api.pendingEmailChange(), retry: false })
  const [email, setEmail] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const { run, dialog } = useReauth()
  const current = session.data?.email ?? ''
  const unverified = session.data !== undefined && !session.data.emailVerifiedAt
  const canEmail = Array.isArray(info.data?.features) && info.data.features.includes('email')

  async function ask(address: string) {
    setBusy(true)
    setError(null)
    try {
      const asked = await run(() => api.requestEmailChange(address))
      if (!asked) return
      setEmail('')
      await queryClient.invalidateQueries({ queryKey: queryKeys.emailChange })
    } catch (caught) {
      if (isApiError(caught) && caught.code === 'account_exists') setError(t('account.exists'))
      else if (isApiError(caught) && caught.code === 'email_not_sent') setError(t('account.emailChange.notSent'))
      else if (isApiError(caught) && caught.status === 429) setError(t('account.emailChange.tooMany'))
      else if (isApiError(caught) && caught.status === 400) setError(t('account.emailChange.invalid'))
      else setError(t('error.generic'))
    } finally {
      setBusy(false)
    }
  }

  function onSubmit(event: FormEvent) {
    event.preventDefault()
    void ask(email.trim())
  }

  if (info.isPending || session.isPending) return null
  return (
    <>
      <section className="card settings-card">
        <h2>{t('account.emailChange.title')}</h2>
        {!canEmail ? (
          <p className="hint">{t('account.emailChange.off')}</p>
        ) : (
          <>
            <p className="hint">{t('account.emailChange.lead')}</p>
            {pending.data ? <p role="status">{t('account.emailChange.pending', { email: pending.data.email, current })}</p> : null}
            {unverified ? (
              <>
                <p className="hint">{t('account.emailChange.unverified')}</p>
                <Button type="button" variant="secondary" disabled={busy} onClick={() => void ask(current)}>
                  {t('account.emailChange.confirmCurrent')}
                </Button>
              </>
            ) : null}
            <form onSubmit={onSubmit}>
              <Field label={t('account.emailChange.new')}>
                <input type="email" required autoComplete="off" value={email} onChange={(event) => setEmail(event.target.value)} />
              </Field>
              {error ? <p className="form-error">{error}</p> : null}
              <button type="submit" disabled={busy}>{t('account.emailChange.send')}</button>
            </form>
          </>
        )}
      </section>
      {dialog}
    </>
  )
}
