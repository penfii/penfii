import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  mfaStatus: vi.fn(),
  mfaSetup: vi.fn(),
  mfaEnable: vi.fn(),
  mfaDisable: vi.fn(),
  mfaRecoveryCodes: vi.fn(),
  reauthPassword: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { TwoFactor } from './TwoFactor'
import { renderWithProviders } from '../../test/renderWithProviders'

const off = { enabled: false, enabledAt: null, recoveryCodesRemaining: 0, setupPending: false }
const on = { enabled: true, enabledAt: '2026-11-20T00:00:00Z', recoveryCodesRemaining: 10, setupPending: false }

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('two-step verification on the account page', () => {
  it('sets it up with a code of the new secret and shows the recovery codes once', async () => {
    api.mfaStatus.mockResolvedValueOnce(off).mockResolvedValue(on)
    api.mfaSetup.mockResolvedValue({ secret: 'ABCD EFGH', otpauthUri: 'otpauth://totp/Penfii%3Aa?secret=ABCDEFGH' })
    api.mfaEnable.mockRejectedValueOnce(new ApiError(401, 'mfa_code_invalid', 'the verification code is not valid')).mockResolvedValue(['aaaa-bbbb-cccc', 'dddd-eeee-ffff'])
    renderWithProviders(<TwoFactor />)
    fireEvent.click(await screen.findByRole('button', { name: 'Set up' }))
    expect(await screen.findByText('ABCD EFGH')).toBeTruthy()
    // The QR code is drawn in the browser (lazy chunk); the key and the link stay as the fallback.
    expect(await screen.findByRole('img', { name: 'QR code for your authenticator app' })).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Open in an authenticator app' }).getAttribute('href')).toBe('otpauth://totp/Penfii%3Aa?secret=ABCDEFGH')

    fireEvent.change(screen.getByLabelText('Code from the app'), { target: { value: '000000' } })
    fireEvent.click(screen.getByRole('button', { name: 'Turn on' }))
    expect(await screen.findByText('That code is not correct')).toBeTruthy()

    fireEvent.change(screen.getByLabelText('Code from the app'), { target: { value: ' 123456 ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Turn on' }))
    expect(await screen.findByText('aaaa-bbbb-cccc')).toBeTruthy()
    expect(api.mfaEnable).toHaveBeenLastCalledWith('123456')
    expect(await screen.findByText(/10 recovery codes left/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'I have saved them' }))
    expect(screen.queryByText('aaaa-bbbb-cccc')).toBeNull()
  })

  it('asks for the ordinary proof first when the server wants a recent one', async () => {
    api.mfaStatus.mockResolvedValue(off)
    api.mfaSetup
      .mockRejectedValueOnce(new ApiError(403, 'reauth_required', 'recent authentication is required', { methods: ['password'] }))
      .mockResolvedValue({ secret: 'ABCD', otpauthUri: 'otpauth://totp/x?secret=ABCD' })
    api.reauthPassword.mockResolvedValue(undefined)
    renderWithProviders(<TwoFactor />)
    fireEvent.click(await screen.findByRole('button', { name: 'Set up' }))
    const password = await screen.findByLabelText('Password')
    fireEvent.change(password, { target: { value: 'my password' } })
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() => expect(api.reauthPassword).toHaveBeenCalledWith('my password'))
    expect(await screen.findByLabelText('Code from the app')).toBeTruthy()
  })

  it('turns it off and replaces the recovery codes only with a code', async () => {
    api.mfaStatus.mockResolvedValue(on)
    api.mfaDisable.mockRejectedValueOnce(new ApiError(401, 'mfa_code_invalid', 'the verification code is not valid')).mockResolvedValue(undefined)
    api.mfaRecoveryCodes.mockResolvedValue(['1111-2222-3333'])
    renderWithProviders(<TwoFactor />)
    fireEvent.click(await screen.findByRole('button', { name: 'Get new recovery codes' }))
    fireEvent.change(screen.getByLabelText('Authenticator or recovery code'), { target: { value: 'aaaa-bbbb-cccc' } })
    fireEvent.click(screen.getByRole('button', { name: 'Get new recovery codes' }))
    expect(await screen.findByText('1111-2222-3333')).toBeTruthy()
    expect(api.mfaRecoveryCodes).toHaveBeenCalledWith('aaaa-bbbb-cccc')

    fireEvent.click(screen.getByRole('button', { name: 'Turn off' }))
    fireEvent.change(screen.getByLabelText('Authenticator or recovery code'), { target: { value: '654321' } })
    fireEvent.click(screen.getByRole('button', { name: 'Turn off' }))
    expect(await screen.findByText('That code is not correct')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Turn off' }))
    await waitFor(() => expect(api.mfaDisable).toHaveBeenCalledTimes(2))
    expect(api.mfaDisable).toHaveBeenLastCalledWith('654321')
  })
})
