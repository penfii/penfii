import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ listInvitations: vi.fn(), revokeInvitation: vi.fn(), reissueInvitation: vi.fn() }))
vi.mock('../../api', () => ({ api }))
vi.mock('../../shell/WorkspaceShell', () => ({ OpenCreate: () => null }))

import type { InvitationCreated } from '@penfii/contracts'
import { InvitationsSection } from './InvitationsSection'
import { InviteResult } from './InviteResult'
import { renderWithProviders } from '../../test/renderWithProviders'

function row(over: object) {
  return {
    id: 'i1', email: 'a@example.com', role: 'MEMBER', status: 'PENDING', expiresAt: '2026-11-10T00:00:00Z', createdAt: '2026-11-03T00:00:00Z',
    createdBy: 'u1', createdByName: 'Owner', acceptedAt: null, revokedAt: null, teamId: null, teamName: null, teamStatus: null,
    emailStatus: null, emailAttemptedAt: null, emailError: null, ...over,
  }
}

const created = (over: Partial<InvitationCreated> = {}): InvitationCreated => ({ id: 'i1', email: 'a@example.com', inviteToken: 'tok-123', reissued: true, emailStatus: 'NOT_CONFIGURED', ...over })

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => { cleanup(); vi.clearAllMocks() })

describe('the e-mail delivery of invitations', () => {
  it('shows how each link was delivered and why a mail failed, without calling a failed mail a failed invitation', async () => {
    api.listInvitations.mockResolvedValue([
      row({ id: 'i1', email: 'sent@example.com', emailStatus: 'SENT' }),
      row({ id: 'i2', email: 'failed@example.com', emailStatus: 'FAILED', emailError: 'the SMTP server refused the message' }),
      row({ id: 'i3', email: 'link@example.com', emailStatus: 'NOT_CONFIGURED' }),
      row({ id: 'i4', email: 'old@example.com' }),
    ])
    renderWithProviders(<InvitationsSection workspaceId="w1" viewerRole="OWNER" />)
    expect(await screen.findByText('Sent')).toBeTruthy()
    expect(screen.getByText('Failed')).toBeTruthy()
    expect(screen.getByText('the SMTP server refused the message')).toBeTruthy()
    expect(screen.getByText('Link only')).toBeTruthy()
    // All four are still pending.
    expect(screen.getAllByText('Pending')).toHaveLength(4)
  })

  it('makes a new link for an open invitation, shows it once and offers it only where the server allows it', async () => {
    api.listInvitations.mockResolvedValue([
      row({ id: 'i1', email: 'a@example.com' }),
      row({ id: 'i2', email: 'late@example.com', status: 'EXPIRED' }),
      row({ id: 'i6', email: 'replaced@example.com', status: 'EXPIRED', revokedAt: '2026-11-11T00:00:00Z' }),
      row({ id: 'i3', email: 'done@example.com', status: 'ACCEPTED' }),
      row({ id: 'i4', email: 'gone@example.com', status: 'REVOKED' }),
      row({ id: 'i5', email: 'admin@example.com', role: 'ADMIN' }),
    ])
    api.reissueInvitation.mockResolvedValue(created({ emailStatus: 'FAILED', emailError: 'TLS to the SMTP server failed' }))
    renderWithProviders(<InvitationsSection workspaceId="w1" viewerRole="ADMIN" />)
    await screen.findByText('a@example.com')
    // Pending and expired ones, for plain members only (an admin does not re-issue an admin invitation).
    expect(screen.getByRole('button', { name: 'New link a@example.com' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'New link late@example.com' })).toBeTruthy()
    // An expired invitation that a newer one replaced is closed.
    expect(screen.queryByRole('button', { name: 'New link replaced@example.com' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'New link done@example.com' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'New link gone@example.com' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'New link admin@example.com' })).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'New link a@example.com' }))
    fireEvent.click(await screen.findByRole('button', { name: 'New link' }))
    await waitFor(() => expect(api.reissueInvitation).toHaveBeenCalledWith('w1', 'i1'))
    const link = (await screen.findByLabelText('Invitation link')) as HTMLInputElement
    expect(link.value).toBe(`${window.location.origin}/invite/tok-123`)
    expect(screen.getByText(/could not be sent \(TLS to the SMTP server failed\)/)).toBeTruthy()
    expect(screen.getByText(/The previous link no longer works/)).toBeTruthy()
  })
})

describe('the invitation link after creating one', () => {
  it('is shown and copied whether or not the server sends e-mail', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    renderWithProviders(<InviteResult invitation={created({ reissued: false })} />)
    expect(screen.getByText('Invitation created.')).toBeTruthy()
    expect(screen.getByText(/does not send e-mail/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(`${window.location.origin}/invite/tok-123`))
    expect(await screen.findByText(/Link copied/)).toBeTruthy()
    cleanup()
    renderWithProviders(<InviteResult invitation={created({ reissued: false, emailStatus: 'SENT' })} />)
    expect(screen.getByText(/e-mail was sent to a@example.com/)).toBeTruthy()
    expect(screen.getByLabelText('Invitation link')).toBeTruthy()
  })
})
