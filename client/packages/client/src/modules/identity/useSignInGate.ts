import { useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { isUnauthorized } from '@penfii/api-client'
import { useLocation, useNavigate } from 'react-router'
import { takeReturn } from '../../app/returnTo'
import { queries } from '../../queries'

/**
 * What the sign-in page does with a person who may already be signed in:
 *
 *  - `checking`: the server has not said yet. A person cached in this tab is not taken as an answer (the session may
 *    have ended or been revoked elsewhere); the page asks every time it opens, and shows its ordinary loading state.
 *  - `redirecting`: the session is valid. The person goes, by replacing this page (so Back does not return to it), to
 *    the place that was remembered for them or the start page.
 *  - `form`: nobody is signed in (no session, an expired or revoked one), so the form is shown.
 *  - `failed`: the server could not say (the network, the server). That is not "signed out": the page says so and offers
 *    to ask again, instead of a form that would fail the same way.
 *
 * Left alone, on purpose: a sign-in that is under way (`signingIn`, which the form reports: the form takes the person on
 * itself, with the place it remembered), and an address with `?auth_error=` (the way a provider sign-in that did not work
 * comes back with its message).
 */
export type SignInGate = {
  phase: 'checking' | 'redirecting' | 'form' | 'failed'
  /** Asks the server again (for `failed`). */
  retry: () => void
  error: unknown
  /** The form tells the page while it is signing in. */
  setSigningIn: (active: boolean) => void
}

export function useSignInGate(): SignInGate {
  const session = useQuery({ ...queries.me(), refetchOnMount: 'always' })
  const navigate = useNavigate()
  const location = useLocation()
  const [signingIn, setSigningIn] = useState(false)
  const left = useRef(false)
  const showsAuthError = new URLSearchParams(location.search).has('auth_error')
  // Only an answer that came after this page opened counts.
  const answered = session.isFetchedAfterMount && !session.isFetching
  const signedIn = answered && session.isSuccess && !signingIn && !showsAuthError

  useEffect(() => {
    if (!signedIn || left.current || !session.data) return
    // The remembered place is used up once and only if it was remembered for this person (see `takeReturn`).
    left.current = true
    void navigate(takeReturn('/', session.data.id), { replace: true })
  }, [signedIn, session.data, navigate])

  let phase: SignInGate['phase']
  if (signingIn || showsAuthError) phase = 'form'
  else if (!answered) phase = 'checking'
  else if (session.isSuccess) phase = 'redirecting'
  else if (session.isError && isUnauthorized(session.error)) phase = 'form'
  else phase = 'failed'
  return { phase, retry: () => void session.refetch(), error: session.error, setSigningIn }
}
