import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  consentRequest: vi.fn(),
  consentDecision: vi.fn(),
  reauthPassword: vi.fn(),
  reauthProviderStart: vi.fn(),
}))
vi.mock('../api', () => ({ api }))

import { ConsentPage } from './ConsentPage'
import { renderWithProviders } from '../test/renderWithProviders'

const view = {
  id: 'r1',
  clientName: 'Codex',
  clientId: 'https://codex.example/c.json',
  redirectHost: '127.0.0.1',
  loopback: true,
  scopes: [
    { scope: 'workspace:read', highRisk: false, change: false },
    { scope: 'ticket:read', highRisk: false, change: false },
    { scope: 'ticket:write', highRisk: false, change: true },
    { scope: 'ticket:delete', highRisk: true, change: true },
  ],
  replacesExisting: false,
  expiresAt: '2026-10-27T00:00:00Z',
}

function mount() {
  return renderWithProviders(<ConsentPage />, { route: '/consent?request=r1' })
}

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('consent page', () => {
  it('shows the client, warns on loopback, leaves risky scopes unchecked and posts only what is checked', async () => {
    api.consentRequest.mockResolvedValue(view)
    api.consentDecision.mockResolvedValue('https://app.example/cb?code=x')
    const assign = vi.fn()
    vi.stubGlobal('location', { ...window.location, assign, pathname: '/consent', search: '?request=r1' })
    mount()
    expect(await screen.findByText('Codex wants to use Penfii')).toBeTruthy()
    expect(screen.getByText(/local address/)).toBeTruthy()
    const [workspace, ticket, write, risky] = screen.getAllByRole('checkbox') as HTMLInputElement[]
    expect(workspace.checked).toBe(true)
    expect(ticket.checked).toBe(true)
    // Changing data and high-risk scopes are never checked for the person.
    expect(write.checked).toBe(false)
    expect(risky.checked).toBe(false)
    fireEvent.click(ticket)
    fireEvent.click(screen.getByRole('button', { name: 'Allow' }))
    await waitFor(() => expect(api.consentDecision).toHaveBeenCalledWith('r1', { approve: true, scopes: ['workspace:read'] }))
    await waitFor(() => expect(assign).toHaveBeenCalledWith('https://app.example/cb?code=x'))
    vi.unstubAllGlobals()
  })

  it('allowing without touching anything grants reading only; ticking write grants it', async () => {
    api.consentRequest.mockResolvedValue(view)
    api.consentDecision.mockResolvedValue('https://app.example/cb?code=x')
    vi.stubGlobal('location', { ...window.location, assign: vi.fn(), pathname: '/consent', search: '?request=r1' })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Allow' }))
    await waitFor(() => expect(api.consentDecision).toHaveBeenLastCalledWith('r1', { approve: true, scopes: ['workspace:read', 'ticket:read'] }))
    cleanup()
    mount()
    const write = (await screen.findAllByRole('checkbox'))[2] as HTMLInputElement
    fireEvent.click(write)
    fireEvent.click(screen.getByRole('button', { name: 'Allow' }))
    await waitFor(() =>
      expect(api.consentDecision).toHaveBeenLastCalledWith('r1', { approve: true, scopes: ['workspace:read', 'ticket:read', 'ticket:write'] }),
    )
    vi.unstubAllGlobals()
  })

  it('deny posts no scopes', async () => {
    api.consentRequest.mockResolvedValue(view)
    api.consentDecision.mockResolvedValue('https://app.example/cb?error=access_denied')
    vi.stubGlobal('location', { ...window.location, assign: vi.fn(), pathname: '/consent', search: '?request=r1' })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Deny' }))
    await waitFor(() => expect(api.consentDecision).toHaveBeenCalledWith('r1', { approve: false, scopes: [] }))
    vi.unstubAllGlobals()
  })

  it('an expired request says so', async () => {
    const { ApiError } = await import('@penfii/api-client')
    api.consentRequest.mockRejectedValue(new ApiError(404, 'not_found', 'not found'))
    mount()
    expect(await screen.findByText('This request has expired')).toBeTruthy()
  })
})
