import { cleanup, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ listInvitations: vi.fn(), revokeInvitation: vi.fn() }))
vi.mock('../../api', () => ({ api }))
vi.mock('../../shell/WorkspaceShell', () => ({ OpenCreate: () => null }))

import { InvitationsSection } from './InvitationsSection'
import { renderWithProviders } from '../../test/renderWithProviders'

function row(over: object) {
  return {
    id: 'i1', email: 'a@example.com', role: 'MEMBER', status: 'PENDING', expiresAt: '2026-11-10T00:00:00Z', createdAt: '2026-11-03T00:00:00Z',
    createdBy: 'u1', createdByName: 'Owner', acceptedAt: null, revokedAt: null, teamId: null, teamName: null, teamStatus: null, ...over,
  }
}

function mount() {
  return renderWithProviders(<InvitationsSection workspaceId="w1" viewerRole="OWNER" />)
}

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => { cleanup(); vi.clearAllMocks() })

describe('the Team of a pending invitation', () => {
  it('names an active Team, says "No Team" otherwise, and explains an archived or missing Team without revoking anything', async () => {
    api.listInvitations.mockResolvedValue([
      row({ id: 'i1', email: 'none@example.com' }),
      row({ id: 'i2', email: 'active@example.com', teamId: 't1', teamName: 'Core', teamStatus: 'ACTIVE' }),
      row({ id: 'i3', email: 'archived@example.com', teamId: 't2', teamName: 'Growth', teamStatus: 'ARCHIVED' }),
      row({ id: 'i4', email: 'gone@example.com', teamId: null, teamName: 'Old', teamStatus: 'GONE' }),
    ])
    mount()
    expect(await screen.findByText('Growth · archived')).toBeTruthy()
    expect(screen.getByText(/Joining now joins the Workspace only; if it is restored before they join/)).toBeTruthy()
    expect(screen.getByText('Old · gone')).toBeTruthy()
    expect(screen.getByText(/no longer exists. Joining now joins the Workspace only/)).toBeTruthy()
    expect(screen.getByText('Core')).toBeTruthy()
    expect(screen.getByText('No Team')).toBeTruthy()
    // Every pending invitation can still be revoked as before: archiving changed nothing about them.
    expect(screen.getAllByRole('button', { name: /^Revoke/ })).toHaveLength(4)
  })
})
