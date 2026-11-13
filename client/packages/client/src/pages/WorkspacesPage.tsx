import { useQuery } from '@tanstack/react-query'
import { Navigate } from 'react-router'
import { api } from '../api'
import { Button, Dialog, EmptyState, Field } from '../design/ui'
import { useI18n } from '../i18n/I18nProvider'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { LAST_WORKSPACE_KEY } from '../shell/account'
import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import { errorText } from '../ui/errorText'
import { LogoutButton } from '../modules/identity/LogoutButton'
import { useSession } from '../modules/identity/useSession'

export function WorkspacesPage() {
  const { t } = useI18n()
  const workspaces = useQuery(queries.workspaces())
  if (workspaces.isPending) return <p className="center-note">{t('loading')}</p>
  if (workspaces.isError) return <p className="center-note">{t('error.generic')}</p>
  if (workspaces.data.length > 0) {
    let stored = ''
    try {
      stored = sessionStorage.getItem(LAST_WORKSPACE_KEY) ?? ''
    } catch {
      stored = ''
    }
    const target = workspaces.data.some((workspace) => workspace.id === stored) ? stored : workspaces.data[0].id
    return <Navigate to={`/w/${target}`} replace />
  }
  return <WorkspaceEmpty />
}

function WorkspaceEmpty() {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const session = useSession().data
  const creation = useQuery(queries.workspaceCreation())
  if (creation.isPending) return <p className="center-note">{t('loading')}</p>
  if (creation.isError) return <p className="center-note">{t('error.generic')}</p>
  // Signing in is not membership: without a workspace the person either starts the instance (nobody exists
  // yet who could invite them) or waits for an invitation.
  if (!creation.data) {
    return (
      <div className="page">
        <EmptyState title={t('workspaces.waiting')} body={t('workspaces.waitingBody', { email: session?.email ?? '' })}>
          <LogoutButton />
        </EmptyState>
      </div>
    )
  }
  return (
    <div className="page">
      <EmptyState title={t('workspaces.empty')} body={t('app.tagline')}>
        <Button onClick={() => setOpen(true)}>{t('workspaces.create')}</Button>
      </EmptyState>
      <WorkspaceCreateDialog open={open} onClose={() => setOpen(false)} />
    </div>
  )
}

function WorkspaceCreateDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const [timeZone, setTimeZone] = useState(Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC')
  const [error, setError] = useState<string | null>(null)
  const zones = Intl.supportedValuesOf('timeZone')
  return (
    <Dialog open={open} title={t('workspaces.create')} onClose={onClose} presentation="full">
      <form
        onSubmit={(event) => {
          event.preventDefault()
          setError(null)
          void api.createWorkspace({ name: name.trim(), timeZone }).then(async (created) => {
            await queryClient.invalidateQueries({ queryKey: queryKeys.workspaces })
            await queryClient.invalidateQueries({ queryKey: queryKeys.workspaceCreation })
            onClose()
            void navigate(`/w/${created.id}`)
          }).catch((caught: unknown) => setError(errorText(caught, t('error.generic'), t('error.dbUnavailable'))))
        }}
      >
        <Field label={t('workspaces.name')}>
          <input required value={name} onChange={(event) => setName(event.target.value)} />
        </Field>
        <Field label={t('workspaces.timeZone')}>
          <select value={timeZone} onChange={(event) => setTimeZone(event.target.value)}>
            {zones.map((zone) => (
              <option key={zone} value={zone}>{zone}</option>
            ))}
          </select>
        </Field>
        {error ? <p className="form-error">{error}</p> : null}
        <Button type="submit">{t('workspaces.create')}</Button>
      </form>
    </Dialog>
  )
}
