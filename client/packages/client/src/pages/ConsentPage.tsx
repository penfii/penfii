import { useEffect, useState } from 'react'
import { isApiError } from '@penfii/api-client'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router'
import { api } from '../api'
import { takeReturn } from '../app/returnTo'
import { useI18n } from '../i18n/I18nProvider'
import type { MessageKey } from '../i18n/messages'
import { useReauth } from '../modules/identity'
import { queries } from '../queries'
import { Button } from '../design/ui'

const SCOPE_LABELS: Record<string, MessageKey> = {
  'workspace:read': 'consent.scope.workspace:read',
  'member:read': 'consent.scope.member:read',
  'search:read': 'consent.scope.search:read',
  'ticket:read': 'consent.scope.ticket:read',
  'ticket:write': 'consent.scope.ticket:write',
  'project:manage': 'consent.scope.project:manage',
}

/**
 * `/consent?request=<id>`: the person decides what a connected application may do. The server
 * validated the request and holds everything that matters; this page only shows it and posts the
 * decision. Scopes can be removed here, never added.
 */
export function ConsentPage() {
  const { t } = useI18n()
  const [params] = useSearchParams()
  const id = params.get('request') ?? ''
  const view = useQuery({ ...queries.oauthRequest(id), enabled: id !== '' })
  const [selected, setSelected] = useState<Set<string> | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { run, dialog } = useReauth()

  // Arrived through the login page: the remembered way back has served its purpose.
  useEffect(() => {
    takeReturn()
  }, [])

  useEffect(() => {
    if (view.data && selected === null) {
      // Only reading starts checked: changing data and high-risk scopes are the person's explicit
      // choice.
      setSelected(new Set(view.data.scopes.filter((scope) => !scope.highRisk && !scope.change).map((scope) => scope.scope)))
    }
  }, [view.data, selected])

  if (id === '' || (view.isError && isApiError(view.error) && view.error.status === 404)) {
    return (
      <main className="page">
        <section className="card">
          <h1>{t('consent.expiredTitle')}</h1>
          <p>{t('consent.expired')}</p>
        </section>
      </main>
    )
  }
  if (!view.data || selected === null) {
    return <p className="center-note">{view.isError ? t('error.generic') : t('loading')}</p>
  }
  const data = view.data
  const reading = data.scopes.filter((scope) => !scope.highRisk && !scope.change)
  const changing = data.scopes.filter((scope) => !scope.highRisk && scope.change)
  const risky = data.scopes.filter((scope) => scope.highRisk)

  function toggle(scope: string) {
    setSelected((current) => {
      const next = new Set(current)
      if (next.has(scope)) next.delete(scope)
      else next.add(scope)
      return next
    })
  }

  async function decide(approve: boolean) {
    setBusy(true)
    setError(null)
    try {
      const target = await run(() => api.consentDecision(id, { approve, scopes: approve ? [...(selected ?? [])] : [] }))
      if (target) window.location.assign(target)
    } catch {
      setError(t('error.generic'))
    } finally {
      setBusy(false)
    }
  }

  const renderScope = (scope: { scope: string }) => (
    <li key={scope.scope}>
      <label>
        <input type="checkbox" checked={selected.has(scope.scope)} onChange={() => toggle(scope.scope)} />
        {SCOPE_LABELS[scope.scope] ? t(SCOPE_LABELS[scope.scope]) : scope.scope}
      </label>
    </li>
  )

  return (
    <main className="page">
      <section className="card consent-card">
        <h1>{t('consent.title', { client: data.clientName })}</h1>
        <p>{t('consent.lead', { host: data.redirectHost })}</p>
        {data.loopback ? <p className="form-error">{t('consent.loopback')}</p> : null}
        {data.replacesExisting ? <p>{t('consent.replaces')}</p> : null}
        <h2>{t('consent.permissions')}</h2>
        {reading.length > 0 ? <ul className="plain-list">{reading.map(renderScope)}</ul> : null}
        {changing.length > 0 ? (
          <>
            <h2>{t('consent.changes')}</h2>
            <p className="field-hint">{t('consent.changesHint')}</p>
            <ul className="plain-list">{changing.map(renderScope)}</ul>
          </>
        ) : null}
        {risky.length > 0 ? (
          <>
            <h2>{t('consent.risky')}</h2>
            <ul className="plain-list">{risky.map(renderScope)}</ul>
          </>
        ) : null}
        <p className="field-hint">{t('consent.note')}</p>
        {dialog}
        {error ? <p className="form-error">{error}</p> : null}
        <div className="row-actions">
          <Button type="button" disabled={busy || selected.size === 0} onClick={() => void decide(true)}>{t('consent.allow')}</Button>
          <Button type="button" variant="secondary" disabled={busy} onClick={() => void decide(false)}>{t('consent.deny')}</Button>
        </div>
      </section>
    </main>
  )
}
