import { StrictMode } from 'react'
import { cleanup, render, screen } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ confirmEmailChange: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { ConfirmEmailForm } from './ConfirmEmail'
import { I18nProvider } from '../../i18n/I18nProvider'

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the e-mail confirmation page', () => {
  it('sends the single-use token once, even when React runs the effect twice', async () => {
    window.history.replaceState(null, '', '/confirm-email#token-once')
    // A second request with the same token would be answered as used.
    api.confirmEmailChange
      .mockResolvedValueOnce(undefined)
      .mockRejectedValue(new ApiError(409, 'email_change_invalid', 'invalid'))
    // Mounted as the app mounts it: StrictMode outermost, which runs effects twice in development.
    render(
      <StrictMode>
        <QueryClientProvider client={new QueryClient()}>
          <I18nProvider>
            <MemoryRouter initialEntries={['/confirm-email']}>
              <ConfirmEmailForm />
            </MemoryRouter>
          </I18nProvider>
        </QueryClientProvider>
      </StrictMode>,
    )
    expect(await screen.findByText(/The address was confirmed/)).toBeTruthy()
    expect(api.confirmEmailChange).toHaveBeenCalledTimes(1)
    expect(api.confirmEmailChange).toHaveBeenCalledWith('token-once')
    // The token is gone from the address bar.
    expect(window.location.hash).toBe('')
  })
})
