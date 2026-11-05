import { useState } from 'react'
import { isApiError } from '@penfii/api-client'
import { useI18n } from '../../i18n/I18nProvider'
import { useReauth } from '../identity'

/**
 * Runs an administrative call: asks for a recent sign-in when the server wants one, then reports a
 * short, readable failure instead of the raw error.
 */
export function useAdminAction() {
  const { t } = useI18n()
  const { run, dialog } = useReauth()
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function perform(action: () => Promise<unknown>, after?: () => Promise<unknown> | void): Promise<boolean> {
    setError(null)
    setBusy(true)
    try {
      const done = await run(async () => {
        await action()
        return true
      })
      if (!done) return false
      await after?.()
      return true
    } catch (caught) {
      setError(failureText(caught, t))
      return false
    } finally {
      setBusy(false)
    }
  }

  return { perform, busy, error, dialog }
}

function failureText(caught: unknown, t: ReturnType<typeof useI18n>['t']): string {
  if (isApiError(caught)) {
    if (caught.code === 'last_owner') return t('admin.lastOwner')
    if (caught.code === 'team_archived') return t('teams.error.archived')
    if (caught.code === 'last_team_manager') return t('admin.teams.lastManager')
    if (caught.code === 'ticket_has_children') return t('admin.trash.hasChildren')
    if (caught.code === 'ticket_has_dependencies') return t('admin.trash.hasDependencies')
  }
  return t('admin.actionError')
}
