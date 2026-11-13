import { useEffect, useState, type FormEvent } from 'react'
import type { InvitationTeamOutcome } from '@penfii/contracts'
import { useQuery } from '@tanstack/react-query'
import { useNavigate } from 'react-router'
import { isApiError } from '@penfii/api-client'
import { api } from '../../api'
import { rememberReturn, takeReturn } from '../../app/returnTo'
import { useI18n } from '../../i18n/I18nProvider'
import { errorText } from '../../ui/errorText'
import { NotFoundState } from '../../ui/NotFoundState'
import { useSession } from './useSession'
import { JoinedCard } from './InviteJoined'
import { ProviderButtons, useAuthRedirectError } from './ProviderButtons'
import { queries } from '../../queries'
import { Button } from '../../design/ui'
import { purgeUserStorage } from '../../app/userBoundary'

export function InviteForm({ token }: { token: string }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const [displayName, setDisplayName] = useState('')
  const [password, setPassword] = useState('')
  const [joined, setJoined] = useState<string | null | undefined>(undefined)
  const [teamOutcome, setTeamOutcome] = useState<InvitationTeamOutcome | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const session = useSession()
  const signedIn = session.data
  const redirectError = useAuthRedirectError()
  const preview = useQuery(queries.invitationPreview(token))

  // Back from signing in (a provider round trip does not pass through the login form): the note is used up.
  useEffect(() => {
    if (signedIn) takeReturn()
  }, [signedIn])

  if (preview.isPending || session.isPending) return <p className="center-note">{t('loading')}</p>
  if (preview.isError) {
    return isApiError(preview.error) && preview.error.status === 404 ? <NotFoundState /> : <p className="center-note">{t('error.generic')}</p>
  }
  const invitation = preview.data
  const forAccount = signedIn?.email.toLowerCase() === invitation.email.toLowerCase()

  async function onSubmit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const accepted = await api.acceptInvitation({
        token,
        displayName: invitation.accountExists && signedIn ? signedIn.displayName : displayName,
        password: signedIn ? undefined : password,
      })
      setTeamOutcome(accepted.team)
      setJoined(accepted.workspaceId)
    } catch (caught) {
      setError(errorText(caught, t('error.generic'), t('error.dbUnavailable')))
    } finally {
      setBusy(false)
    }
  }

  // The invitation is kept for this tab while the person signs in, then this page opens again.
  function signInToAccept() {
    rememberReturn(`/invite/${token}`)
    void navigate('/login')
  }

  function switchAccount() {
    rememberReturn(`/invite/${token}`)
    purgeUserStorage()
    void api.logout().finally(() => window.location.assign(`/invite/${token}`))
  }

  if (joined !== undefined) {
    return <JoinedCard outcome={teamOutcome} onContinue={() => void navigate(joined ? `/w/${joined}` : '/')} />
  }

  const lead = <p className="hint">{t('invite.for', { email: invitation.email, workspace: invitation.workspaceName })}</p>

  if (signedIn && !forAccount) {
    return (
      <div className="card">
        <h1>{t('invite.title')}</h1>
        {lead}
        <p>{t('invite.wrongAccount', { email: invitation.email, current: signedIn.email })}</p>
        <Button type="button" variant="secondary" onClick={switchAccount}>
          {t('invite.switchAccount')}
        </Button>
      </div>
    )
  }

  if (!signedIn && invitation.accountExists) {
    return (
      <div className="card">
        <h1>{t('invite.title')}</h1>
        {lead}
        <p>{t('invite.signInFirst', { email: invitation.email })}</p>
        <Button type="button" onClick={signInToAccept}>
          {t('invite.signIn')}
        </Button>
      </div>
    )
  }

  return (
    <form className="card" onSubmit={(event) => void onSubmit(event)}>
      <h1>{t('invite.title')}</h1>
      {lead}
      {signedIn ? (
        <p className="hint">{t('invite.asSignedIn', { email: signedIn.email })}</p>
      ) : (
        <>
          <label>
            {t('invite.displayName')}
            <input required value={displayName} onChange={(event) => setDisplayName(event.target.value)} />
          </label>
          <label>
            {t('invite.password')}
            <input type="password" autoComplete="new-password" required minLength={15} value={password} onChange={(event) => setPassword(event.target.value)} />
          </label>
          <small className="hint">{t('password.rule')}</small>
        </>
      )}
      {error ?? redirectError ? <p className="form-error">{error ?? redirectError}</p> : null}
      <Button type="submit" disabled={busy}>
        {t('invite.submit')}
      </Button>
      {signedIn ? null : <ProviderButtons intent="invite" invitationToken={token} returnTo="/invite/joined" />}
    </form>
  )
}
