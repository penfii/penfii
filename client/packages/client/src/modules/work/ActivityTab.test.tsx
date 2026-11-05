import { cleanup, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getTicketActivity: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { ActivityTab } from './ActivityTab'
import { renderWithProviders } from '../../test/renderWithProviders'

const entry = (id: string, via?: string) => ({
  id,
  ticketId: 't1',
  revisionNo: 2,
  activityKind: 'ticket.updated',
  actorUserId: 'u1',
  actorName: 'Alice',
  data: {},
  changes: [],
  occurredAt: '2026-10-27T01:00:00Z',
  ...(via ? { via } : {}),
})

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('activity tab', () => {
  it('marks work done through an MCP client and leaves the person’s own work unmarked', async () => {
    api.getTicketActivity.mockResolvedValue({ items: [entry('a', 'Codex'), entry('b')], nextCursor: null })
    renderWithProviders(<ActivityTab ticketId="t1" />)
    expect(await screen.findByText('via Codex')).toBeTruthy()
    expect(screen.getAllByText(/Alice/)).toHaveLength(2)
    expect(screen.getAllByText(/^via /)).toHaveLength(1)
  })
})
