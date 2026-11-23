import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  serverInfo: vi.fn(),
  me: vi.fn(),
  pendingEmailChange: vi.fn(),
  requestEmailChange: vi.fn(),
  confirmEmailChange: vi.fn(),
  reauthPassword: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { ConfirmEmailForm } from './ConfirmEmail'
import { EmailChange } from './EmailChange'
import { renderWithProviders } from '../../test/renderWithProviders'

const me = { id: 'u', email: 'old@example.com', displayName: 'Old', emailVerifiedAt: '2026-01-01T00:00:00Z' }

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.me.mockResolvedValue(me)
  api.pendingEmailChange.mockResolvedValue(null)
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  window.history.replaceState(null, '', '/')
})

describe('changing the e-mail address', () => {
  it('without server e-mail it says so and offers no form', async () => {
    api.serverInfo.mockResolvedValue({ features: ['chat'] })
    renderWithProviders(<EmailChange />)
    expect(await screen.findByText(/sends no e-mail/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Send confirmation link' })).toBeNull()
  })

  it('asks for the link, asks the person to confirm when it must, and keeps the current address', async () => {
    api.serverInfo.mockResolvedValue({ features: ['email'] })
    api.requestEmailChange
      .mockRejectedValueOnce(new ApiError(403, 'reauth_required', 'x', { methods: ['password'] }))
      .mockResolvedValueOnce({ email: 'new@example.com', expiresAt: '2026-01-02T00:00:00Z' })
    api.reauthPassword.mockResolvedValue(undefined)
    renderWithProviders(<EmailChange />)
    fireEvent.change(await screen.findByLabelText('New e-mail address'), { target: { value: 'new@example.com' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send confirmation link' }))
    fireEvent.change(await screen.findByLabelText('Password'), { target: { value: 'secret' } })
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() => expect(api.requestEmailChange).toHaveBeenCalledTimes(2))
    expect(api.requestEmailChange).toHaveBeenLastCalledWith('new@example.com')
  })

  it('shows a waiting change with the address that still signs in', async () => {
    api.serverInfo.mockResolvedValue({ features: ['email'] })
    api.pendingEmailChange.mockResolvedValue({ email: 'new@example.com', expiresAt: '2026-01-02T00:00:00Z' })
    renderWithProviders(<EmailChange />)
    expect(await screen.findByText(/Waiting for you to open the link sent to new@example\.com.*old@example\.com/)).toBeTruthy()
  })

  it('an address an operator set offers to confirm itself', async () => {
    api.serverInfo.mockResolvedValue({ features: ['email'] })
    api.me.mockResolvedValue({ ...me, emailVerifiedAt: null })
    api.requestEmailChange.mockResolvedValue({ email: 'old@example.com', expiresAt: '2026-01-02T00:00:00Z' })
    renderWithProviders(<EmailChange />)
    fireEvent.click(await screen.findByRole('button', { name: 'Confirm this address' }))
    await waitFor(() => expect(api.requestEmailChange).toHaveBeenCalledWith('old@example.com'))
  })
})

describe('the confirmation link page', () => {
  it('uses the token from the fragment once and removes it from the address bar', async () => {
    window.history.replaceState(null, '', '/confirm-email#abc123')
    api.confirmEmailChange.mockResolvedValue(undefined)
    renderWithProviders(<ConfirmEmailForm />, { route: '/confirm-email' })
    expect(await screen.findByText(/The address was confirmed/)).toBeTruthy()
    expect(api.confirmEmailChange).toHaveBeenCalledWith('abc123')
    expect(window.location.hash).toBe('')
  })

  it('says when the link no longer works, and when there is no token at all', async () => {
    window.history.replaceState(null, '', '/confirm-email#used')
    api.confirmEmailChange.mockRejectedValue(new ApiError(409, 'email_change_token_invalid', 'x'))
    renderWithProviders(<ConfirmEmailForm />, { route: '/confirm-email' })
    expect(await screen.findByText(/no longer works/)).toBeTruthy()
    cleanup()
    window.history.replaceState(null, '', '/confirm-email')
    api.confirmEmailChange.mockClear()
    renderWithProviders(<ConfirmEmailForm />, { route: '/confirm-email' })
    expect(await screen.findByText(/no longer works/)).toBeTruthy()
    expect(api.confirmEmailChange).not.toHaveBeenCalled()
  })
})
