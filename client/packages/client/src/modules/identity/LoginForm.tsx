import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isApiError, isDbUnavailable } from '@penfii/api-client'
import type { Me } from '@penfii/contracts'
import { Link, useNavigate } from 'react-router'
import { api } from '../../api'
import { peekReturn, takeReturn } from '../../app/returnTo'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { ProviderButtons, useAuthRedirectError } from './ProviderButtons'
import { forgetEmail, keepSignedInChoice, lastMethod, rememberEmail, rememberedEmail, setKeepSignedInChoice, setLastMethod } from './loginMemory'

const DAY = 86_400

/** Where the keyboard goes after a sign-in that did not work: the field to correct, or the button that tries again. */
type FocusTarget = 'password' | 'code' | 'submit'

/** Whether focus is still where the request left it, or nowhere: only then does it move by itself. */
function focusIsFree(origin: Element | null, submit: Element | null): boolean {
  const active = document.activeElement
  return active === null || active === document.body || active === document.documentElement || active === origin || active === submit
}

/** Whole minutes (at least 1) until the lock ends, from the server's `retryAfterSeconds`. */
function retryMinutes(error: { details?: unknown }): number {
  const seconds = (error.details as { retryAfterSeconds?: unknown } | undefined)?.retryAfterSeconds
  return typeof seconds === 'number' && seconds > 0 ? Math.max(1, Math.ceil(seconds / 60)) : 15
}

export function LoginForm({
  onSigningIn,
}: {
  /** Told while a sign-in is under way: the page around the form must not take the person on before the form does. */
  onSigningIn?: (active: boolean) => void
} = {}) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  // The e-mail address the person chose to keep, restored with its checkbox; never a password.
  const [kept] = useState(rememberedEmail)
  const [email, setEmail] = useState(kept ?? '')
  const [password, setPassword] = useState('')
  const [rememberId, setRememberId] = useState(kept !== null)
  const [keepSignedIn, setKeepSignedIn] = useState(keepSignedInChoice)
  // The second step of an account with two-step verification: the server asked for a code after the password.
  const [needsCode, setNeedsCode] = useState(false)
  const [code, setCode] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // A second submit while one is under way is refused here too, before React has drawn the disabled button.
  const inFlight = useRef(false)
  const passwordRef = useRef<HTMLInputElement>(null)
  const codeRef = useRef<HTMLInputElement>(null)
  const submitRef = useRef<HTMLButtonElement>(null)
  // The element that had focus when the request was made, and where focus goes once a failed request has finished.
  const origin = useRef<Element | null>(null)
  const refocus = useRef<FocusTarget | null>(null)
  const redirectError = useAuthRedirectError()
  // The link appears only while the server sends e-mail; without it an operator sets the password (`admin-cli`).
  const info = useQuery({ ...queries.serverInfo(), retry: false })
  const canReset = Array.isArray(info.data?.features) && info.data.features.includes('password-reset')
  // "Keep me signed in" is offered only while the server has it on (the policy also says for how long).
  const options = useQuery(queries.authOptions())
  const longSession = options.data?.longSession.enabled ? options.data.longSession : null
  const wantsLongSession = longSession !== null && keepSignedIn
  const lastUsed = lastMethod() === 'password'
  const failure = error ?? redirectError

  // After a failure the form is drawn again with the button enabled (and the code field, if it was just asked for); focus then
  // goes to where the person acts next, unless they have moved on to some other control while the request was under way.
  useEffect(() => {
    const target = refocus.current
    if (busy || !target) return
    refocus.current = null
    if (!focusIsFree(origin.current, submitRef.current)) return
    const field = target === 'password' ? passwordRef.current : target === 'code' ? codeRef.current : submitRef.current
    field?.focus()
  }, [busy])

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (inFlight.current) return
    // What the fields hold now, not what React last heard: a browser or a password manager that fills the form
    // without an input event still signs in with the filled values.
    const form = new FormData(event.currentTarget)
    const typedEmail = String(form.get('email') ?? email)
    const typedPassword = String(form.get('password') ?? password)
    const typedCode = needsCode ? String(form.get('code') ?? code).trim() : ''
    inFlight.current = true
    onSigningIn?.(true)
    origin.current = document.activeElement
    setBusy(true)
    setError(null)
    let signedIn = false
    try {
      await api.login({
        email: typedEmail,
        password: typedPassword,
        ...(needsCode ? { code: typedCode } : {}),
        ...(wantsLongSession ? { keepSignedIn: true } : {}),
      })
      // Only a sign-in that worked changes what is remembered: this e-mail if the box is ticked, and the method.
      if (rememberId) rememberEmail(typedEmail)
      setLastMethod('password')
      await queryClient.invalidateQueries({ queryKey: queryKeys.me })
      signedIn = true
      void navigate(takeReturn('/', queryClient.getQueryData<Me>(queryKeys.me)?.id))
    } catch (caught) {
      if (isApiError(caught) && caught.code === 'mfa_required') {
        setNeedsCode(true)
        setCode('')
        refocus.current = 'code'
      } else if (isApiError(caught) && caught.status === 401) {
        setError(t(needsCode ? 'login.codeFailed' : 'login.failed'))
        // The code that was refused is retyped; a refused ID or password is corrected in the password field.
        refocus.current = needsCode ? 'code' : 'password'
      } else {
        if (isApiError(caught) && caught.status === 429) setError(t('login.tooManyAttempts', { minutes: String(retryMinutes(caught)) }))
        else if (isDbUnavailable(caught)) setError(t('error.dbUnavailable'))
        else setError(t('error.generic'))
        // Nothing to correct: the same form is sent again (by the person, never by itself).
        refocus.current = 'submit'
      }
    } finally {
      inFlight.current = false
      // A sign-in that worked leaves this page on its own: the page around the form must not take the person anywhere else.
      if (!signedIn) onSigningIn?.(false)
      setBusy(false)
    }
  }

  return (
    <form className="card" onSubmit={(event) => void onSubmit(event)}>
      <h1>{t('login.title')}</h1>
      {peekReturn()?.startsWith('/invite/') ? <p className="hint">{t('login.forInvitation')}</p> : null}
      <label>
        {t('login.email')}
        <input id="login-email" name="email" type="email" autoComplete="username" required aria-describedby={failure ? 'login-error' : undefined} value={email} onChange={(event) => setEmail(event.target.value)} />
      </label>
      <label>
        {t('login.password')}
        <input
          id="login-password"
          name="password"
          type="password"
          autoComplete="current-password"
          required
          ref={passwordRef}
          aria-describedby={failure ? 'login-error' : undefined}
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
      </label>
      {needsCode ? (
        <>
          <p className="hint">{t('login.codeLead')}</p>
          <label>
            {t('login.code')}
            <input name="code" autoComplete="one-time-code" required ref={codeRef} aria-describedby={failure ? 'login-error' : undefined} value={code} onChange={(event) => setCode(event.target.value)} />
          </label>
        </>
      ) : null}
      <div className="login-options">
        <label className="login-check">
          <input
            type="checkbox"
            name="rememberEmail"
            checked={rememberId}
            onChange={(event) => {
              setRememberId(event.target.checked)
              // Unticking removes the kept address at once; ticking keeps nothing until a sign-in works.
              if (!event.target.checked) forgetEmail()
            }}
          />
          <span>{t('login.rememberEmail')}</span>
        </label>
        {longSession ? (
          <label className="login-check">
            <input
              type="checkbox"
              name="keepSignedIn"
              checked={keepSignedIn}
              onChange={(event) => {
                setKeepSignedIn(event.target.checked)
                setKeepSignedInChoice(event.target.checked)
              }}
            />
            <span>
              {t('login.keepSignedIn')}
              <small className="hint">
                {t('login.keepSignedInHint', {
                  days: String(Math.max(1, Math.round(longSession.maxAgeSeconds / DAY))),
                  idleDays: String(Math.max(1, Math.round(Math.min(longSession.idleSeconds, longSession.maxAgeSeconds) / DAY))),
                })}
              </small>
            </span>
          </label>
        ) : null}
      </div>
      {failure ? <p id="login-error" className="form-error" role="alert">{failure}</p> : null}
      <button type="submit" ref={submitRef} disabled={busy} aria-describedby={failure ? 'login-error' : undefined}>
        {t('login.submit')}
        {lastUsed ? <span className="last-used">{t('login.lastUsed')}</span> : null}
      </button>
      {needsCode ? (
        <button type="button" className="secondary" onClick={() => { setNeedsCode(false); setCode(''); setError(null) }}>
          {t('login.back')}
        </button>
      ) : null}
      {canReset ? <Link to="/forgot-password">{t('login.forgot')}</Link> : null}
      <ProviderButtons intent="login" returnTo={peekReturn()} keepSignedIn={wantsLongSession} />
    </form>
  )
}
