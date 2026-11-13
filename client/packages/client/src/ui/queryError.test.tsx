import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), listWorkspaces: vi.fn(), identities: vi.fn(), sessions: vi.fn(), serverInfo: vi.fn(), getProjectPlanning: vi.fn() }))
vi.mock('../api', () => ({ api }))

import { ProjectsPage } from '../pages/ProjectsPage'
import { SignInMethods } from '../modules/identity/SignInMethods'
import { QueryError } from './QueryError'
import { renderWithProviders } from '../test/renderWithProviders'

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('a failed read', () => {
  it('on the Project page shows the error and a retry, and the page once the retry works', async () => {
    api.getSpace.mockRejectedValueOnce(new ApiError(503, 'db_unavailable', 'db'))
    api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p1', name: 'Penfii App', teamIds: [], participating: true }], channels: [], dms: [] })
    api.getProjectPlanning.mockResolvedValue({ canManage: false, sprints: [], milestones: [] })
    renderWithProviders(<ProjectsPage />, { route: '/w/w/projects/p1', path: '/w/:workspaceId/projects/:projectId' })
    // Not "loading" for ever, and not an empty page: what went wrong, and a way to try again.
    expect(await screen.findByText(/database is temporarily unavailable/i)).toBeTruthy()
    expect(screen.queryByText('Loading…')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    await waitFor(() => expect(api.getSpace).toHaveBeenCalledTimes(2))
    expect(await screen.findByRole('heading', { name: 'Penfii App' })).toBeTruthy()
    expect(screen.queryByText(/temporarily unavailable/i)).toBeNull()
  })

  it('in the sign-in methods says so instead of showing a method list with nothing in it', async () => {
    api.identities.mockRejectedValue(new ApiError(500, 'internal', 'boom'))
    api.sessions.mockResolvedValue({ items: [], reauthMethods: [], reauthWindowSeconds: 0 })
    renderWithProviders(<SignInMethods />, { route: '/' })
    expect(await screen.findByRole('alert')).toBeTruthy()
  })

  it('names the database being unavailable', () => {
    renderWithProviders(<QueryError query={{ error: new ApiError(503, 'db_unavailable', 'x'), refetch: () => undefined }} />, { route: '/' })
    expect(screen.getByRole('alert').textContent).toContain('temporarily unavailable')
  })
})
