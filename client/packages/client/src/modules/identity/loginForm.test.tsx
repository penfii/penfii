import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ login: vi.fn(), authOptions: vi.fn(), authStart: vi.fn(), sessions: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { LoginForm } from './LoginForm'
import { renderWithProviders } from '../../test/renderWithProviders'

function mount() {
  return renderWithProviders(<LoginForm />, { route: '/' })
}

async function submit(password: string) {
  fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'dana@example.com' } })
  fireEvent.change(document.querySelector('input[type=password]')!, { target: { value: password } })
  fireEvent.click(screen.getByRole('button', { name: 'Log in' }))
}

const NO_LONG_SESSION = { enabled: false, maxAgeSeconds: 0, idleSeconds: 0 }
const LONG_SESSION = { enabled: true, maxAgeSeconds: 30 * 86_400, idleSeconds: 7 * 86_400 }

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
  localStorage.setItem('penfii.locale', 'en-US')
  api.authOptions.mockResolvedValue({ providers: [], longSession: NO_LONG_SESSION })
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('sign-in form', () => {
  it('applies no password rule: a short password is sent to the server as typed', async () => {
    api.login.mockRejectedValue(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    mount()
    const input = document.querySelector('input[type=password]') as HTMLInputElement
    expect(input.minLength).toBe(-1)
    await submit('1234')
    expect(await screen.findByText(/incorrect/i)).toBeTruthy()
    expect(api.login).toHaveBeenCalledWith({ email: 'dana@example.com', password: '1234' })
  })

  it('says it is a lock, and for how long, when the server answers 429', async () => {
    api.login.mockRejectedValue(new ApiError(429, 'rate_limited', 'too many attempts, try again later', { retryAfterSeconds: 610 }))
    mount()
    await submit('whatever')
    expect(await screen.findByText('Too many attempts. Try again in 11 minutes')).toBeTruthy()
  })

  it('falls back to 15 minutes when the server gives no time', async () => {
    api.login.mockRejectedValue(new ApiError(429, 'rate_limited', 'x'))
    mount()
    await submit('whatever')
    expect(await screen.findByText('Too many attempts. Try again in 15 minutes')).toBeTruthy()
  })

  it('asks to try again later when the database is unavailable, and not for any other 503', async () => {
    api.login.mockRejectedValue(new ApiError(503, 'db_unavailable', 'the database is temporarily unavailable'))
    mount()
    await submit('whatever')
    expect(await screen.findByText('The database is temporarily unavailable. Try again in a moment')).toBeTruthy()
    cleanup()
    api.login.mockRejectedValue(new ApiError(503, 'search_unavailable', 'search is down'))
    mount()
    await submit('whatever')
    expect(await screen.findByText('The request could not be completed')).toBeTruthy()
  })
})

describe('sign-in with two-step verification', () => {
  it('asks for the code after the password, sends it with the same password, and can go back', async () => {
    api.login.mockRejectedValueOnce(new ApiError(401, 'mfa_required', 'a verification code is required'))
    mount()
    await submit('right-password')
    expect(await screen.findByLabelText('Authenticator or recovery code')).toBeTruthy()
    expect(screen.queryByText(/incorrect/i)).toBeNull()

    api.login.mockRejectedValueOnce(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    fireEvent.change(screen.getByLabelText('Authenticator or recovery code'), { target: { value: ' 123456 ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Log in' }))
    expect(await screen.findByText('The code is incorrect or was already used')).toBeTruthy()
    expect(api.login).toHaveBeenLastCalledWith({ email: 'dana@example.com', password: 'right-password', code: '123456' })

    fireEvent.click(screen.getByRole('button', { name: 'Back' }))
    expect(screen.queryByLabelText('Authenticator or recovery code')).toBeNull()
  })

  it('does not ask for a code when the password is wrong', async () => {
    api.login.mockRejectedValue(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    mount()
    await submit('wrong')
    expect(await screen.findByText(/incorrect/i)).toBeTruthy()
    expect(screen.queryByLabelText('Authenticator or recovery code')).toBeNull()
  })
})

describe('password managers and autofill (FEA-180)', () => {
  it('names the fields and keeps the autocomplete meaning a password manager looks for', () => {
    mount()
    const email = screen.getByLabelText('Email') as HTMLInputElement
    const password = document.querySelector('input[type=password]') as HTMLInputElement
    expect([email.name, email.autocomplete, email.type]).toEqual(['email', 'username', 'email'])
    expect([password.name, password.autocomplete]).toEqual(['password', 'current-password'])
    expect(email.form).toBe(password.form)
    expect(email.form!.querySelector('button[type=submit]')).toBeTruthy()
  })

  it('signs in with what the fields hold even when no input event reached React (a filled form)', async () => {
    api.login.mockResolvedValue(undefined)
    mount()
    // A password manager that sets the value without the events React listens to.
    ;(screen.getByLabelText('Email') as HTMLInputElement).value = 'filled@example.com'
    ;(document.querySelector('input[type=password]') as HTMLInputElement).value = 'filled-password'
    fireEvent.click(screen.getByRole('button', { name: /^Log in/ }))
    await waitFor(() => expect(api.login).toHaveBeenCalledWith({ email: 'filled@example.com', password: 'filled-password' }))
  })

  it('sends the same filled password again with the code on the second step', async () => {
    api.login.mockRejectedValueOnce(new ApiError(401, 'mfa_required', 'a verification code is required'))
    mount()
    await submit('right-password')
    const code = await screen.findByLabelText('Authenticator or recovery code')
    expect((code as HTMLInputElement).name).toBe('code')
    api.login.mockResolvedValueOnce(undefined)
    fireEvent.change(code, { target: { value: '123456' } })
    fireEvent.click(screen.getByRole('button', { name: /^Log in/ }))
    await waitFor(() => expect(api.login).toHaveBeenLastCalledWith({ email: 'dana@example.com', password: 'right-password', code: '123456' }))
  })
})

describe('remembering the e-mail address (FEA-181)', () => {
  const box = () => screen.getByRole('checkbox', { name: 'Remember my ID' }) as HTMLInputElement

  it('starts off, stores nothing for a failed sign-in, and keeps the address only after a successful one', async () => {
    api.login.mockRejectedValueOnce(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    mount()
    expect(box().checked).toBe(false)
    fireEvent.click(box())
    await submit('wrong')
    await screen.findByText(/incorrect/i)
    expect(localStorage.getItem('penfii.login.email')).toBeNull()

    api.login.mockResolvedValueOnce(undefined)
    fireEvent.click(screen.getByRole('button', { name: /^Log in/ }))
    await waitFor(() => expect(localStorage.getItem('penfii.login.email')).toBe('dana@example.com'))
    expect(localStorage.getItem('penfii.login.method')).toBe('password')
    // Nothing secret is ever written.
    expect(JSON.stringify({ ...localStorage })).not.toContain('wrong')
  })

  it('restores the address and the ticked box, and unticking removes the address', () => {
    localStorage.setItem('penfii.login.email', 'kept@example.com')
    mount()
    expect(box().checked).toBe(true)
    expect((screen.getByLabelText('Email') as HTMLInputElement).value).toBe('kept@example.com')
    fireEvent.click(box())
    expect(localStorage.getItem('penfii.login.email')).toBeNull()
  })

  it('does not store the address when the box is off, and replaces it with the one that worked', async () => {
    api.login.mockResolvedValue(undefined)
    mount()
    await submit('right')
    await waitFor(() => expect(api.login).toHaveBeenCalled())
    expect(localStorage.getItem('penfii.login.email')).toBeNull()
    cleanup()
    localStorage.setItem('penfii.login.email', 'old@example.com')
    mount()
    fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'new@example.com' } })
    fireEvent.change(document.querySelector('input[type=password]')!, { target: { value: 'pw' } })
    fireEvent.click(screen.getByRole('button', { name: /^Log in/ }))
    await waitFor(() => expect(localStorage.getItem('penfii.login.email')).toBe('new@example.com'))
  })

  it('does not remember the address while a second step is pending, only after the code worked', async () => {
    api.login.mockRejectedValueOnce(new ApiError(401, 'mfa_required', 'a verification code is required'))
    mount()
    fireEvent.click(box())
    await submit('right')
    await screen.findByLabelText('Authenticator or recovery code')
    expect(localStorage.getItem('penfii.login.email')).toBeNull()
    api.login.mockResolvedValueOnce(undefined)
    fireEvent.change(screen.getByLabelText('Authenticator or recovery code'), { target: { value: '123456' } })
    fireEvent.click(screen.getByRole('button', { name: /^Log in/ }))
    await waitFor(() => expect(localStorage.getItem('penfii.login.email')).toBe('dana@example.com'))
  })

  it('still signs in when storage is blocked', async () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked')
    })
    try {
      api.login.mockResolvedValue(undefined)
      mount()
      // The language is unreadable too, so the fields are found by name rather than by label.
      fireEvent.click(document.querySelector('input[name=rememberEmail]')!)
      fireEvent.change(document.querySelector('input[name=email]')!, { target: { value: 'dana@example.com' } })
      fireEvent.change(document.querySelector('input[name=password]')!, { target: { value: 'right' } })
      fireEvent.click(document.querySelector('button[type=submit]')!)
      await waitFor(() => expect(api.login).toHaveBeenCalledWith({ email: 'dana@example.com', password: 'right' }))
      expect(document.querySelector('.form-error')).toBeNull()
    } finally {
      getItem.mockRestore()
      setItem.mockRestore()
    }
  })

  it('shows the last used method only while it is offered', async () => {
    localStorage.setItem('penfii.login.method', 'google')
    api.authOptions.mockResolvedValue({ providers: [{ key: 'google', displayName: 'Google', kind: 'oidc' }], longSession: NO_LONG_SESSION })
    mount()
    const google = await screen.findByRole('button', { name: /Continue with Google/ })
    expect(google.textContent).toContain('Last used')
    expect(screen.getByRole('button', { name: /^Log in/ }).textContent).not.toContain('Last used')
    cleanup()
    // The provider is gone: no badge anywhere, and the password button does not claim it either.
    api.authOptions.mockResolvedValue({ providers: [{ key: 'github', displayName: 'GitHub', kind: 'github' }], longSession: NO_LONG_SESSION })
    mount()
    const github = await screen.findByRole('button', { name: /Continue with GitHub/ })
    expect(github.textContent).not.toContain('Last used')
    expect(document.body.textContent).not.toContain('Last used')
    cleanup()
    localStorage.setItem('penfii.login.method', 'password')
    mount()
    expect(screen.getByRole('button', { name: /^Log in/ }).textContent).toContain('Last used')
  })

  it('records a provider only when the signed-in session says it was that provider', async () => {
    const { confirmProviderLogin } = await import('./confirmLogin')
    const { markProviderAttempt } = await import('./loginMemory')
    // A pressed button alone records nothing.
    api.authStart.mockResolvedValue('https://idp.example/auth')
    expect(localStorage.getItem('penfii.login.method')).toBeNull()
    markProviderAttempt('google')
    api.sessions.mockResolvedValue({ items: [{ id: '1', current: true, authMethod: 'password', createdAt: new Date().toISOString() }] })
    await confirmProviderLogin()
    expect(localStorage.getItem('penfii.login.method')).toBeNull()
    // The note was used up: a later load does not reuse it.
    api.sessions.mockResolvedValue({ items: [{ id: '1', current: true, authMethod: 'google', createdAt: new Date().toISOString() }] })
    await confirmProviderLogin()
    expect(localStorage.getItem('penfii.login.method')).toBeNull()
    // The session came from that provider after the button: recorded.
    markProviderAttempt('google')
    await confirmProviderLogin()
    expect(localStorage.getItem('penfii.login.method')).toBe('google')
    // A session that is older than the button is not a result of it.
    localStorage.clear()
    markProviderAttempt('google')
    api.sessions.mockResolvedValue({ items: [{ id: '1', current: true, authMethod: 'google', createdAt: new Date(Date.now() - 3_600_000).toISOString() }] })
    await confirmProviderLogin()
    expect(localStorage.getItem('penfii.login.method')).toBeNull()
  })
})

describe('keep me signed in (FEA-182)', () => {
  const keep = () => screen.queryByRole('checkbox', { name: /Keep me signed in/ }) as HTMLInputElement | null

  it('is offered only while the server offers it, off the first time, and says what it means', async () => {
    mount()
    await waitFor(() => expect(api.authOptions).toHaveBeenCalled())
    expect(keep()).toBeNull()
    cleanup()
    api.authOptions.mockResolvedValue({ providers: [], longSession: LONG_SESSION })
    mount()
    const box = await screen.findByRole('checkbox', { name: /Keep me signed in/ })
    expect((box as HTMLInputElement).checked).toBe(false)
    expect(document.body.textContent).toContain('up to 30 days')
    expect(document.body.textContent).toContain('not used for 7 days')
  })

  it('asks for the long session only when ticked, and remembers the choice in this browser', async () => {
    api.authOptions.mockResolvedValue({ providers: [], longSession: LONG_SESSION })
    api.login.mockResolvedValue(undefined)
    mount()
    await submit('pw')
    await waitFor(() => expect(api.login).toHaveBeenLastCalledWith({ email: 'dana@example.com', password: 'pw' }))
    fireEvent.click(await screen.findByRole('checkbox', { name: /Keep me signed in/ }))
    expect(localStorage.getItem('penfii.login.keepSignedIn')).toBe('1')
    fireEvent.click(screen.getByRole('button', { name: /^Log in/ }))
    await waitFor(() => expect(api.login).toHaveBeenLastCalledWith({ email: 'dana@example.com', password: 'pw', keepSignedIn: true }))
    // The choice is restored next time, and unticking is remembered as off.
    cleanup()
    mount()
    expect((await screen.findByRole('checkbox', { name: /Keep me signed in/ }) as HTMLInputElement).checked).toBe(true)
    fireEvent.click(keep()!)
    expect(localStorage.getItem('penfii.login.keepSignedIn')).toBe('0')
  })

  it('is independent of remembering the address', async () => {
    api.authOptions.mockResolvedValue({ providers: [], longSession: LONG_SESSION })
    api.login.mockResolvedValue(undefined)
    mount()
    fireEvent.click(await screen.findByRole('checkbox', { name: /Keep me signed in/ }))
    await submit('pw')
    await waitFor(() => expect(api.login).toHaveBeenCalledWith({ email: 'dana@example.com', password: 'pw', keepSignedIn: true }))
    expect(localStorage.getItem('penfii.login.email')).toBeNull()
  })

  it('is not sent when the server turned it off, even if the browser remembers it', async () => {
    localStorage.setItem('penfii.login.keepSignedIn', '1')
    api.login.mockResolvedValue(undefined)
    mount()
    await waitFor(() => expect(api.authOptions).toHaveBeenCalled())
    expect(keep()).toBeNull()
    await submit('pw')
    await waitFor(() => expect(api.login).toHaveBeenCalledWith({ email: 'dana@example.com', password: 'pw' }))
  })

  it('goes with the provider start of a sign-in, never an invitation', async () => {
    api.authOptions.mockResolvedValue({ providers: [{ key: 'google', displayName: 'Google', kind: 'oidc' }], longSession: LONG_SESSION })
    api.authStart.mockReturnValue(new Promise(() => undefined))
    mount()
    fireEvent.click(await screen.findByRole('checkbox', { name: /Keep me signed in/ }))
    fireEvent.click(await screen.findByRole('button', { name: /Continue with Google/ }))
    await waitFor(() => expect(api.authStart).toHaveBeenCalledWith('google', expect.objectContaining({ intent: 'login', keepSignedIn: true })))
    // A press is not a success: nothing about the method is recorded yet.
    expect(localStorage.getItem('penfii.login.method')).toBeNull()
  })
})

describe('focus after a sign-in that did not work (FEA-194)', () => {
  const passwordField = () => document.querySelector('input[type=password]') as HTMLInputElement
  const submitButton = () => screen.getByRole('button', { name: /^Log in/ })

  /** Fills the form and presses the button the way a mouse does: the button has focus when the request starts. */
  async function pressButton(password: string) {
    fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'dana@example.com' } })
    fireEvent.change(passwordField(), { target: { value: password } })
    submitButton().focus()
    fireEvent.click(submitButton())
  }

  it('puts the keyboard in the password field when the ID or password was refused, and says so as an alert', async () => {
    api.login.mockRejectedValue(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    mount()
    await pressButton('wrong')
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/incorrect/i)
    await waitFor(() => expect(document.activeElement).toBe(passwordField()))
    // What was typed is still there, and the field is described by the message.
    expect(passwordField().value).toBe('wrong')
    expect((screen.getByLabelText('Email') as HTMLInputElement).value).toBe('dana@example.com')
    expect(passwordField().getAttribute('aria-describedby')).toBe(alert.id)
  })

  it('does it again for every refusal, not only the first', async () => {
    api.login.mockRejectedValue(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    mount()
    fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'dana@example.com' } })
    fireEvent.change(passwordField(), { target: { value: 'wrong' } })
    for (let attempt = 0; attempt < 3; attempt += 1) {
      submitButton().focus()
      fireEvent.click(submitButton())
      await waitFor(() => expect(document.activeElement).toBe(passwordField()))
      expect(api.login).toHaveBeenCalledTimes(attempt + 1)
      ;(document.activeElement as HTMLElement).blur()
    }
  })

  it('puts the keyboard in the code field when the code is asked for, and again when it was refused', async () => {
    api.login.mockRejectedValueOnce(new ApiError(401, 'mfa_required', 'a verification code is required'))
    mount()
    await pressButton('right-password')
    const code = await screen.findByLabelText('Authenticator or recovery code')
    await waitFor(() => expect(document.activeElement).toBe(code))
    // The refused code is the next thing to retype; the password and the ID stay.
    api.login.mockRejectedValueOnce(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    fireEvent.change(code, { target: { value: '000000' } })
    submitButton().focus()
    fireEvent.click(submitButton())
    expect(await screen.findByText('The code is incorrect or was already used')).toBeTruthy()
    await waitFor(() => expect(document.activeElement).toBe(code))
    expect((code as HTMLInputElement).value).toBe('000000')
  })

  it('returns to the button that sends the form again when the server or the network failed, or it is a lock', async () => {
    for (const failure of [
      new ApiError(503, 'db_unavailable', 'down'),
      new ApiError(500, 'internal', 'broken'),
      new ApiError(429, 'rate_limited', 'later', { retryAfterSeconds: 60 }),
      new TypeError('Failed to fetch'),
    ]) {
      api.login.mockRejectedValueOnce(failure)
      const view = mount()
      await pressButton('whatever')
      await screen.findByRole('alert')
      await waitFor(() => expect(document.activeElement).toBe(submitButton()))
      expect(submitButton().hasAttribute('disabled')).toBe(false)
      // It was not sent again by itself.
      expect(api.login).toHaveBeenCalledTimes(1)
      api.login.mockClear()
      view.unmount()
    }
  })

  it('does not take the keyboard from a control the person went to while the request was under way', async () => {
    let fail: (error: unknown) => void = () => undefined
    api.login.mockImplementation(() => new Promise((_, reject) => { fail = reject }))
    mount()
    await pressButton('wrong')
    // They go on to something else: the e-mail field (to fix the address) or the link to reset the password.
    const email = screen.getByLabelText('Email') as HTMLInputElement
    email.focus()
    fail(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    await screen.findByRole('alert')
    await Promise.resolve()
    expect(document.activeElement).toBe(email)
  })

  it('refuses a second submit while one is under way', async () => {
    api.login.mockImplementation(() => new Promise(() => undefined))
    mount()
    await pressButton('slow')
    fireEvent.submit(submitButton().closest('form')!)
    fireEvent.submit(submitButton().closest('form')!)
    expect(api.login).toHaveBeenCalledTimes(1)
    expect(submitButton().hasAttribute('disabled')).toBe(true)
  })

  it('leaves the keyboard alone after a sign-in that worked', async () => {
    api.login.mockResolvedValue(undefined)
    mount()
    await pressButton('right')
    await waitFor(() => expect(api.login).toHaveBeenCalled())
    expect(screen.queryByRole('alert')).toBeNull()
  })
})
