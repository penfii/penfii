import { lazy, Suspense, useState, type FormEvent } from 'react'
import { isApiError } from '@penfii/api-client'
import type { MfaSetup } from '@penfii/api-client'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { Button, Field } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { queryKeys } from '../../queryKeys'
import { QueryError } from '../../ui/QueryError'
import { useReauth } from './reauth'

// The QR encoder is its own chunk: only this screen needs it, and the first load must not carry it.
const MfaQr = lazy(() => import('./MfaQr'))

type Step = 'idle' | 'setup' | 'disable' | 'regenerate'

/**
 * Account → two-step verification (an authenticator app, TOTP). Turning it on needs a recent proof and a
 * code of the new secret; turning it off and new recovery codes need the proof and a current code. The
 * recovery codes are shown once, right after they are made.
 */
export function TwoFactor() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const status = useQuery({ ...queries.mfaStatus() })
  const { run, dialog } = useReauth()
  const [step, setStep] = useState<Step>('idle')
  const [setup, setSetup] = useState<MfaSetup | null>(null)
  const [code, setCode] = useState('')
  const [codes, setCodes] = useState<string[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const enabled = status.data?.enabled === true

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.mfa })
    await queryClient.invalidateQueries({ queryKey: queryKeys.sessions })
  }

  function describe(caught: unknown): string {
    if (isApiError(caught) && caught.code === 'mfa_code_invalid') return t('mfa.wrongCode')
    if (isApiError(caught) && caught.status === 429) return t('mfa.tooMany')
    return t('error.generic')
  }

  function leave() {
    setStep('idle')
    setSetup(null)
    setCode('')
    setError(null)
  }

  async function start() {
    setBusy(true)
    setError(null)
    try {
      const started = await run(() => api.mfaSetup())
      if (!started) return
      setSetup(started)
      setCode('')
      setStep('setup')
    } catch (caught) {
      setError(isApiError(caught) && caught.code === 'mfa_already_enabled' ? t('mfa.alreadyOn') : t('error.generic'))
    } finally {
      setBusy(false)
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    const typed = code.trim()
    try {
      if (step === 'setup') {
        const made = await run(() => api.mfaEnable(typed))
        if (!made) return
        setCodes(made)
        leave()
      } else if (step === 'regenerate') {
        const made = await run(() => api.mfaRecoveryCodes(typed))
        if (!made) return
        setCodes(made)
        leave()
      } else if (step === 'disable') {
        const done = await run(() => api.mfaDisable(typed).then(() => true))
        if (!done) return
        setCodes(null)
        leave()
      }
      await refresh()
    } catch (caught) {
      setError(describe(caught))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="card settings-card account-mfa">
      <h2>{t('mfa.title')}</h2>
      {dialog}
      {status.isError ? <QueryError query={status} compact /> : null}
      {codes ? (
        <div role="status" className="mfa-codes">
          <p><b>{t('mfa.codesTitle')}</b></p>
          <p className="hint">{t('mfa.codesLead')}</p>
          <ul className="plain-list">
            {codes.map((recovery) => (
              <li key={recovery}><code>{recovery}</code></li>
            ))}
          </ul>
          <Button type="button" variant="secondary" onClick={() => setCodes(null)}>{t('mfa.codesSaved')}</Button>
        </div>
      ) : null}
      {status.data && step === 'idle' ? (
        enabled ? (
          <>
            <p>{t('mfa.on')} · {t('mfa.remaining', { count: String(status.data.recoveryCodesRemaining) })}</p>
            <p className="hint">{t('mfa.providerNote')}</p>
            <div className="row-actions">
              <Button type="button" variant="secondary" onClick={() => { setError(null); setStep('regenerate') }}>{t('mfa.newCodes')}</Button>
              <Button type="button" variant="secondary" onClick={() => { setError(null); setStep('disable') }}>{t('mfa.turnOff')}</Button>
            </div>
          </>
        ) : (
          <>
            <p className="hint">{t('mfa.lead')}</p>
            <Button type="button" variant="secondary" disabled={busy} onClick={() => void start()}>{t('mfa.setUp')}</Button>
          </>
        )
      ) : null}
      {step === 'setup' && setup ? (
        <form onSubmit={(event) => void submit(event)}>
          <p>{t('mfa.setupLead')}</p>
          <Suspense fallback={null}>
            <MfaQr uri={setup.otpauthUri} />
          </Suspense>
          <p><code className="mfa-secret">{setup.secret}</code></p>
          <p><a href={setup.otpauthUri}>{t('mfa.openApp')}</a></p>
          <Field label={t('mfa.codeFromApp')}>
            <input inputMode="numeric" autoComplete="one-time-code" autoFocus required value={code} onChange={(event) => setCode(event.target.value)} />
          </Field>
          {error ? <p className="form-error">{error}</p> : null}
          <div className="row-actions">
            <button type="submit" disabled={busy}>{t('mfa.turnOn')}</button>
            <Button type="button" variant="secondary" onClick={leave}>{t('reauth.cancel')}</Button>
          </div>
        </form>
      ) : null}
      {step === 'disable' || step === 'regenerate' ? (
        <form onSubmit={(event) => void submit(event)}>
          <p className="hint">{step === 'disable' ? t('mfa.disableLead') : t('mfa.newCodesLead')}</p>
          <Field label={t('mfa.codeOrRecovery')}>
            <input autoComplete="one-time-code" autoFocus required value={code} onChange={(event) => setCode(event.target.value)} />
          </Field>
          {error ? <p className="form-error">{error}</p> : null}
          <div className="row-actions">
            <button type="submit" disabled={busy}>{step === 'disable' ? t('mfa.turnOff') : t('mfa.newCodes')}</button>
            <Button type="button" variant="secondary" onClick={leave}>{t('reauth.cancel')}</Button>
          </div>
        </form>
      ) : null}
      {step === 'idle' && error ? <p className="form-error">{error}</p> : null}
    </section>
  )
}
