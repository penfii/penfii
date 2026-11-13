import { useState } from 'react'
import { QueryError } from '../../ui/QueryError'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { useReauth } from './reauth'
import { queries } from '../../queries'
import { queryKeys } from '../../queryKeys'

/** Account → connected apps (MCP clients). Shown only when this server has MCP turned on. */
export function ConnectedApps() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const info = useQuery({ ...queries.serverInfo(), retry: false })
  const enabled = Array.isArray(info.data?.features) && info.data.features.includes('mcp')
  const grants = useQuery({ ...queries.oauthGrants(), enabled })
  const { run, dialog } = useReauth()
  const [error, setError] = useState<string | null>(null)

  if (!enabled) return null

  async function act(action: () => Promise<void>) {
    setError(null)
    try {
      await run(action)
      await queryClient.invalidateQueries({ queryKey: queryKeys.oauthGrants })
    } catch {
      setError(t('error.generic'))
    }
  }

  const items = grants.data ?? []
  return (
    <section className="card settings-card">
      <h2>{t('apps.title')}</h2>
      <p className="field-hint">{t('apps.lead')}</p>
      {dialog}
      {error ? <p className="form-error">{error}</p> : null}
      {grants.isError ? <QueryError query={grants} compact /> : null}
      {grants.isSuccess && items.length === 0 ? <p>{t('apps.none')}</p> : null}
      <ul className="plain-list">
        {items.map((grant) => (
          <li key={grant.id}>
            <strong>{grant.clientName}</strong>
            {' · '}
            {grant.scopes.join(', ')}
            {' · '}
            {grant.lastUsedAt ? t('apps.lastUsed', { at: new Date(grant.lastUsedAt).toLocaleString() }) : t('apps.neverUsed')}
            <button type="button" className="secondary" onClick={() => void act(() => api.revokeOAuthGrant(grant.id))}>{t('apps.disconnect')}</button>
          </li>
        ))}
      </ul>
      {items.length > 1 ? (
        <button type="button" className="secondary" onClick={() => void act(() => api.revokeAllOAuthGrants())}>{t('apps.disconnectAll')}</button>
      ) : null}
    </section>
  )
}
