import { useEffect, useState } from 'react'
import type { McpPolicyInput } from '@penfii/api-client'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import type { MessageKey } from '../../i18n/messages'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { useReauth } from './reauth'
import { Button } from '../../design/ui'

const FLAGS: { key: keyof McpPolicyInput; label: MessageKey }[] = [
  { key: 'mcpEnabled', label: 'mcpPolicy.enabled' },
  { key: 'allowWrite', label: 'mcpPolicy.write' },
  { key: 'allowDestructive', label: 'mcpPolicy.destructive' },
  { key: 'allowAdministrative', label: 'mcpPolicy.administrative' },
]

/**
 * Workspace MCP Policy: a switch per class of MCP capability. It can only restrict; it does not
 * grant anything. Everyone in the Workspace can read it, owners and admins can change it.
 */
export function McpPolicySettings({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const info = useQuery({ ...queries.serverInfo(), retry: false })
  const enabled = Array.isArray(info.data?.features) && info.data.features.includes('mcp')
  const policy = useQuery({ ...queries.mcpPolicy(workspaceId), enabled })
  const workspaces = useQuery(queries.workspaces())
  const role = workspaces.data?.find((workspace) => workspace.id === workspaceId)?.role
  const canEdit = role === 'OWNER' || role === 'ADMIN'
  const [draft, setDraft] = useState<McpPolicyInput | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const { run, dialog } = useReauth()

  useEffect(() => {
    if (policy.data) {
      const { mcpEnabled, allowWrite, allowDestructive, allowAdministrative } = policy.data
      setDraft({ mcpEnabled, allowWrite, allowDestructive, allowAdministrative })
    }
  }, [policy.data])

  if (!enabled || !draft) return null
  const current = draft

  async function save() {
    setError(null)
    setSaved(false)
    try {
      const updated = await run(() => api.setMcpPolicy(workspaceId, current))
      if (!updated) return
      setSaved(true)
      await queryClient.invalidateQueries({ queryKey: queryKeys.mcpPolicy(workspaceId) })
    } catch {
      setError(t('error.generic'))
    }
  }

  return (
    <section className="card settings-card">
      <h2>{t('mcpPolicy.title')}</h2>
      <p className="field-hint">{t('mcpPolicy.lead')}</p>
      {policy.data?.isDefault ? <p className="field-hint">{t('mcpPolicy.isDefault')}</p> : null}
      <ul className="plain-list">
        {FLAGS.map((flag) => (
          <li key={flag.key}>
            <label>
              <input
                type="checkbox"
                disabled={!canEdit}
                checked={current[flag.key]}
                onChange={(event) => setDraft({ ...current, [flag.key]: event.target.checked })}
              />
              {t(flag.label)}
            </label>
          </li>
        ))}
      </ul>
      {dialog}
      {error ? <p className="form-error">{error}</p> : null}
      {saved ? <p>{t('account.saved')}</p> : null}
      {canEdit ? (
        <Button type="button" onClick={() => void save()}>{t('account.save')}</Button>
      ) : (
        <p className="field-hint">{t('mcpPolicy.readOnly')}</p>
      )}
    </section>
  )
}
