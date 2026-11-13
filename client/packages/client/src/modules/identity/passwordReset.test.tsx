import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'

const api = vi.hoisted(() => ({
  requestPasswordReset: vi.fn(), confirmPasswordReset: vi.fn(), login: vi.fn(), serverInfo: vi.fn(),
  listAuthProviders: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { ForgotPasswordForm, ResetPasswordForm } from './PasswordReset'
import { LoginForm } from './LoginForm'
import { renderWithProviders } from '../../test/renderWithProviders'

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.listAuthProviders.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  window.history.replaceState(null, '', '/')
})

describe('asking for a reset mail', () => {
  it('says the same thing whatever the address, and never that an account exists', async () => {
    api.requestPasswordReset.mockResolvedValue(undefined)
    renderWithProviders(<ForgotPasswordForm />, { route: '/' })
    fireEvent.change(screen.getByLabelText('Email'), { target: { value: ' who@example.com ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send reset link' }))
    await waitFor(() => expect(api.requestPasswordReset).toHaveBeenCalledWith('who@example.com'))
    expect(await screen.findByText(/If there is an account for that address/)).toBeTruthy()
  })

  it('tells people to ask an administrator when the server sends no e-mail', async () => {
    api.requestPasswordReset.mockRejectedValue(new ApiError(404, 'not_found', 'not found'))
    renderWithProviders(<ForgotPasswordForm />, { route: '/' })
    fireEvent.change(screen.getByLabelText('Email'), { target: { value: 'who@example.com' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send reset link' }))
    expect(await screen.findByText(/sends no e-mail/)).toBeTruthy()
  })
})

describe('the reset link page', () => {
  it('takes the token from the fragment, removes it from the address and sends it in the body only', async () => {
    window.history.replaceState(null, '', '/reset-password#secret-token')
    api.confirmPasswordReset.mockResolvedValue(undefined)
    renderWithProviders(<ResetPasswordForm />, { route: '/' })
    await waitFor(() => expect(window.location.hash).toBe(''))
    expect(window.location.href).not.toContain('secret-token')
    fireEvent.change(screen.getByLabelText('New password'), { target: { value: 'a long enough passphrase' } })
    fireEvent.change(screen.getByLabelText('New password again'), { target: { value: 'a long enough passphrase' } })
    fireEvent.click(screen.getByRole('button', { name: 'Set password' }))
    await waitFor(() => expect(api.confirmPasswordReset).toHaveBeenCalledWith('secret-token', 'a long enough passphrase'))
    expect(await screen.findByText(/Your password was changed/)).toBeTruthy()
  })

  it('does not send two different passwords, and explains a link that no longer works', async () => {
    window.history.replaceState(null, '', '/reset-password#old-token')
    api.confirmPasswordReset.mockRejectedValue(new ApiError(409, 'reset_token_invalid', 'invalid'))
    renderWithProviders(<ResetPasswordForm />, { route: '/' })
    fireEvent.change(screen.getByLabelText('New password'), { target: { value: 'a long enough passphrase' } })
    fireEvent.change(screen.getByLabelText('New password again'), { target: { value: 'another long passphrase!' } })
    fireEvent.click(screen.getByRole('button', { name: 'Set password' }))
    expect(await screen.findByText('The two passwords are not the same.')).toBeTruthy()
    expect(api.confirmPasswordReset).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('New password again'), { target: { value: 'a long enough passphrase' } })
    fireEvent.click(screen.getByRole('button', { name: 'Set password' }))
    expect(await screen.findByText(/no longer works/)).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Get a new reset link' })).toBeTruthy()
  })

  it('opened without a token, only offers a new link', () => {
    renderWithProviders(<ResetPasswordForm />, { route: '/' })
    expect(screen.getByText(/no longer works/)).toBeTruthy()
    expect(api.confirmPasswordReset).not.toHaveBeenCalled()
  })
})

describe('the way into the reset page', () => {
  it('is offered on the login form only while the server sends e-mail', async () => {
    api.serverInfo.mockResolvedValue({ apiVersion: 'x', features: ['mcp'], limits: {} })
    renderWithProviders(<LoginForm />, { route: '/' })
    await waitFor(() => expect(api.serverInfo).toHaveBeenCalled())
    expect(screen.queryByRole('link', { name: 'Forgot your password?' })).toBeNull()
    cleanup()
    api.serverInfo.mockResolvedValue({ apiVersion: 'x', features: ['email', 'password-reset'], limits: {} })
    renderWithProviders(<LoginForm />, { route: '/' })
    expect(await screen.findByRole('link', { name: 'Forgot your password?' })).toBeTruthy()
  })
})
