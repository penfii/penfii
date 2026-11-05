import { cleanup, fireEvent, screen } from '@testing-library/react'
import { Route, Routes } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  previewInvitation: vi.fn(),
  acceptInvitation: vi.fn(),
  me: vi.fn(),
  authProviders: vi.fn(),
  logout: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { InviteForm } from './InviteForm'
import { peekReturn } from '../../app/returnTo'
import { renderWithProviders } from '../../test/renderWithProviders'

function mount() {
  return renderWithProviders(<Routes>
      <Route path="/invite/:token" element={<InviteForm token="tok" />} />
      <Route path="/login" element={<p>login page</p>} />
      <Route path="/w/:id" element={<p>inside workspace</p>} />
      </Routes>, { route: '/invite/tok' })
}

const preview = { email: 'dana@example.com', workspaceName: 'Acme', role: 'MEMBER', accountExists: true }

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  sessionStorage.clear()
  api.authProviders.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('invitation link', () => {
  it('an existing account signed out is sent to sign in and the invitation is kept', async () => {
    api.previewInvitation.mockResolvedValue(preview)
    api.me.mockRejectedValue(Object.assign(new Error('x'), { status: 401 }))
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Sign in to accept' }))
    expect(await screen.findByText('login page')).toBeTruthy()
    expect(peekReturn()).toBe('/invite/tok')
    expect(screen.queryByText(/already exists/i)).toBeNull()
  })

  it('the signed-in invited account accepts and enters the workspace; membership comes only from accepting', async () => {
    api.previewInvitation.mockResolvedValue(preview)
    api.me.mockResolvedValue({ id: 'u', email: 'Dana@example.com', displayName: 'Dana' })
    api.acceptInvitation.mockResolvedValue({ workspaceId: 'w1', team: null })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Join' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Continue' }))
    expect(await screen.findByText('inside workspace')).toBeTruthy()
    expect(api.acceptInvitation).toHaveBeenCalledWith({ token: 'tok', displayName: 'Dana', password: undefined })
  })

  it.each([
    ['JOINED', /joined the Core Team as a member/],
    ['ARCHIVED', /archived or no longer exists, so you were not added/],
    ['GONE', /archived or no longer exists, so you were not added/],
  ] as const)('says what became of the invitation\'s Team: %s', async (status, text) => {
    api.previewInvitation.mockResolvedValue(preview)
    api.me.mockResolvedValue({ id: 'u', email: 'dana@example.com', displayName: 'Dana' })
    api.acceptInvitation.mockResolvedValue({ workspaceId: 'w1', team: { teamId: status === 'GONE' ? null : 't1', teamName: 'Core', status } })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Join' }))
    expect(await screen.findByText(text)).toBeTruthy()
    // Joining the Workspace succeeded either way.
    expect(screen.getByText(/You joined\./)).toBeTruthy()
  })

  it('another signed-in account cannot accept it', async () => {
    api.previewInvitation.mockResolvedValue(preview)
    api.me.mockResolvedValue({ id: 'b', email: 'bob@example.com', displayName: 'Bob' })
    mount()
    expect(await screen.findByText(/signed in as bob@example\.com/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Join' })).toBeNull()
    expect(api.acceptInvitation).not.toHaveBeenCalled()
  })

  it('a new e-mail still gets the sign-up form', async () => {
    api.previewInvitation.mockResolvedValue({ ...preview, accountExists: false })
    api.me.mockRejectedValue(Object.assign(new Error('x'), { status: 401 }))
    mount()
    expect(await screen.findByLabelText('Password')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Join' })).toBeTruthy()
  })

  it('tells the password rule, and lets a password manager fill it and a paste in', async () => {
    api.previewInvitation.mockResolvedValue({ ...preview, accountExists: false })
    api.me.mockRejectedValue(Object.assign(new Error('x'), { status: 401 }))
    mount()
    const input = (await screen.findByLabelText('Password')) as HTMLInputElement
    expect(screen.getByText(/At least 15 and at most 128 characters/)).toBeTruthy()
    expect(input.minLength).toBe(15)
    expect(input.autocomplete).toBe('new-password')
    // A paste is not intercepted: no handler stops the event.
    const paste = new Event('paste', { bubbles: true, cancelable: true })
    input.dispatchEvent(paste)
    expect(paste.defaultPrevented).toBe(false)
    // Nothing caps the length below the server's 128 (a maxLength would cut a pasted passphrase silently).
    expect(input.maxLength).toBe(-1)
  })
})
