import { useEffect, useState, type FormEvent } from 'react'
import { useLocation, useParams } from 'react-router'
import { isApiError } from '@penfii/api-client'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../api'
import { errorText } from '../ui/errorText'
import { Button, Field } from '../design/ui'
import { applyTheme, readTheme, saveTheme, type ThemeMode } from '../design/theme'
import { NotificationSettings } from '../modules/attention/NotificationSettings'
import { LocaleToggle, useI18n } from '../i18n/I18nProvider'
import { ConnectedApps, SignInMethods, useReauth, useSession } from '../modules/identity'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { ResponsiveShell } from '../shell/ResponsiveShell'
import { LAST_WORKSPACE_KEY } from '../shell/account'
import { ProfileSettings } from './ProfileSettings'

export function SettingsPage() {
  const { pathname } = useLocation()
  const { workspaceId = '' } = useParams()
  if (pathname.endsWith('/notifications')) return <div className="page-scroll settings-page"><NotificationSettings /></div>
  if (pathname.endsWith('/appearance')) return <AppearanceSettings />
  if (pathname.endsWith('/apps')) return <div className="page-scroll settings-page"><ConnectedApps /></div>
  if (pathname.endsWith('/account')) {
    return <AccountScreen />
  }
  return <ProfileSettings workspaceId={workspaceId} />
}

function AppearanceSettings() {
  const { t } = useI18n()
  const [mode, setMode] = useState<ThemeMode>(readTheme)
  useEffect(() => applyTheme(mode), [mode])
  return (
    <div className="page-scroll settings-page">
      <header className="page-intro">
        <h1>{t('settings.appearance')}</h1>
        <p className="hint">{t('appearance.lead')}</p>
      </header>
      <section className="card settings-card">
        <h2>{t('appearance.language')}</h2>
        <LocaleToggle />
        <h2>{t('appearance.theme')}</h2>
        <div className="row-actions theme-choices">
          {([
            ['system', 'appearance.system'],
            ['light', 'appearance.light'],
            ['dark', 'appearance.dark'],
          ] as const).map(([value, label]) => (
            <Button
              key={value}
              type="button"
              variant={mode === value ? 'primary' : 'secondary'}
              onClick={() => {
                setMode(value)
                saveTheme(value)
              }}
            >
              {t(label)}
            </Button>
          ))}
        </div>
      </section>
    </div>
  )
}

/** The sign-in details, the sign-in methods and the signed-in devices (frame 5.2). */
function AccountScreen() {
  const { t } = useI18n()
  return (
    <div className="page-scroll account-page">
      <header className="page-intro">
        <h1>{t('settings.account')}</h1>
        <p className="hint">{t('account.lead')}</p>
      </header>
      <div className="account-grid">
        <AccountForm />
        <SignInMethods />
      </div>
    </div>
  )
}

export function AccountPage() {
  const workspaces = useQuery(queries.workspaces())
  let stored = ''
  try {
    stored = sessionStorage.getItem(LAST_WORKSPACE_KEY) ?? ''
  } catch {
    stored = ''
  }
  const workspaceId = workspaces.data?.some((workspace) => workspace.id === stored) ? stored : workspaces.data?.[0]?.id
  const form = <AccountScreen />
  if (!workspaceId) return <div className="page">{form}</div>
  return <ResponsiveShell section="settings" workspaceId={workspaceId} desktopOnly>{form}</ResponsiveShell>
}

function AccountForm() {
  const { t } = useI18n()
  const session = useSession()
  const queryClient = useQueryClient()
  const [displayName, setDisplayName] = useState('')
  const [email, setEmail] = useState('')
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const [busy, setBusy] = useState(false)
  const { run, dialog } = useReauth()

  useEffect(() => {
    if (!session.data) return
    setDisplayName(session.data.displayName)
    setEmail(session.data.email)
  }, [session.data])

  async function onSubmit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    setSaved(false)
    try {
      // The password is optional now: without it the server asks for a recent proof when needed.
      const saved = await run(() =>
        api.updateMe({
          displayName,
          email,
          currentPassword: currentPassword === '' ? undefined : currentPassword,
          newPassword: newPassword.trim() === '' ? null : newPassword,
        }),
      )
      if (!saved) return
      setCurrentPassword('')
      setNewPassword('')
      setSaved(true)
      await queryClient.invalidateQueries({ queryKey: queryKeys.me })
    } catch (caught) {
      if (isApiError(caught) && caught.status === 401) {
        // The 401 may also mean the session ended; asking who I am hands that case to the sign-in redirect.
        const stillSignedIn = await api.me().then(() => true, () => false)
        if (stillSignedIn) setError(t('account.wrongPassword'))
      }
      else if (isApiError(caught) && caught.code === 'account_exists') setError(t('account.exists'))
      else if (isApiError(caught) && caught.code === 'validation') setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
      else setError(t('error.generic'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
    <form className="card settings-card" onSubmit={(event) => void onSubmit(event)}>
      <h2>{t('account.loginInfo')}</h2>
      <Field label={t('account.email')}>
        <input type="email" required autoComplete="username" value={email} onChange={(event) => setEmail(event.target.value)} />
      </Field>
      <Field label={t('account.currentPassword')} hint={t('account.currentPasswordHint')}>
        <input type="password" autoComplete="current-password" value={currentPassword} onChange={(event) => setCurrentPassword(event.target.value)} />
      </Field>
      <Field label={t('account.newPassword')} hint={`${t('password.rule')} ${t('account.newPasswordHint')}`}>
        <input type="password" autoComplete="new-password" minLength={15} value={newPassword} onChange={(event) => setNewPassword(event.target.value)} />
      </Field>
      {error ? <p className="form-error">{error}</p> : null}
      {saved ? <p>{t('account.saved')}</p> : null}
      <button type="submit" disabled={busy}>{t('account.save')}</button>
    </form>
    {/* Outside the form: the dialog has a form of its own, and a form inside a form does not submit. */}
    {dialog}
    </>
  )
}
