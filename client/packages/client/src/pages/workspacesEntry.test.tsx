import { cleanup, screen } from '@testing-library/react'
import { Route, Routes } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  listWorkspaces: vi.fn(),
  workspaceCreationOpen: vi.fn(),
  me: vi.fn(),
  logout: vi.fn(),
  createWorkspace: vi.fn(),
}))
vi.mock('../api', () => ({ api }))

import { WorkspacesPage } from './WorkspacesPage'
import { renderWithProviders } from '../test/renderWithProviders'

function mount() {
  return renderWithProviders(<Routes>
      <Route path="/" element={<WorkspacesPage />} />
      <Route path="/w/:id" element={<p>inside workspace</p>} />
      </Routes>, { route: '/' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.me.mockResolvedValue({ id: 'u1', email: 'dana@example.com', displayName: 'Dana' })
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('entry without a workspace', () => {
  it('a member goes straight into a workspace and never asks whether creation is open', async () => {
    api.listWorkspaces.mockResolvedValue([{ id: 'w1', name: 'W', timeZone: 'UTC', role: 'MEMBER' }])
    mount()
    expect(await screen.findByText('inside workspace')).toBeTruthy()
    expect(api.workspaceCreationOpen).not.toHaveBeenCalled()
  })

  it('on an instance without any workspace the person can create the first one', async () => {
    api.listWorkspaces.mockResolvedValue([])
    api.workspaceCreationOpen.mockResolvedValue(true)
    mount()
    expect(await screen.findByRole('button', { name: 'Create workspace' })).toBeTruthy()
    expect(screen.queryByText('Waiting for an invitation')).toBeNull()
  })

  it('once a workspace exists the person without membership waits for an invitation and cannot create one', async () => {
    api.listWorkspaces.mockResolvedValue([])
    api.workspaceCreationOpen.mockResolvedValue(false)
    mount()
    expect(await screen.findByText('Waiting for an invitation')).toBeTruthy()
    expect(screen.getByText(/dana@example\.com/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Create workspace' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeTruthy()
  })
})
