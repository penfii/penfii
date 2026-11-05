import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { Route, Routes } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '../queryKeys'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), listWorkspaces: vi.fn() }))
const open = vi.hoisted(() => vi.fn())
vi.mock('../api', () => ({ api }))
vi.mock('./create', () => ({ useCreate: () => open }))

import { ContextSidebar } from './ContextSidebar'
import { renderWithProviders } from '../test/renderWithProviders'

function mount(path: string, section: string) {
  return renderWithProviders(
    <Routes>
      <Route path="/w/:workspaceId/projects/:projectId/*" element={<ContextSidebar workspaceId="w" section={section} />} />
      <Route path="/w/:workspaceId/*" element={<ContextSidebar workspaceId="w" section={section} />} />
    </Routes>,
    { route: path },
  ).client
}

const workspaces = (role: string) => [{ id: 'w', name: 'Ourspace', role }]
const space = (canManage: boolean) => ({
  teams: [],
  projects: [{ id: 'p1', name: 'Penfii App', teamIds: [], participating: true, canManage }],
  channels: [],
  dms: [],
})

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => {
  cleanup()
  open.mockClear()
})

describe('Create actions follow who the server lets create', () => {
  it.each([
    ['MEMBER', false],
    ['ADMIN', true],
    ['OWNER', true],
  ])('offers a Workspace channel to %s: %s', async (role, offered) => {
    api.listWorkspaces.mockResolvedValue(workspaces(role))
    api.getSpace.mockResolvedValue(space(false))
    mount('/w/w/chat', 'chat')
    fireEvent.click(await screen.findByRole('button', { name: /\+ New/ }))
    await screen.findByRole('menuitem', { name: 'New direct message' })
    if (offered) await screen.findByRole('menuitem', { name: 'New channel' })
    else expect(screen.queryByRole('menuitem', { name: 'New channel' })).toBeNull()
  })

  it.each([
    [true, true],
    [false, false],
  ])('offers a Project channel to the Project admin only (canManage %s)', async (canManage, offered) => {
    // A Workspace owner who does not manage the Project is not offered it.
    api.listWorkspaces.mockResolvedValue(workspaces('OWNER'))
    api.getSpace.mockResolvedValue(space(canManage))
    const client = mount('/w/w/projects/p1/chat', 'projects')
    await waitFor(() => expect(client.getQueryData(queryKeys.space('w'))).toBeTruthy())
    expect(screen.queryByRole('button', { name: 'Create channel' }) !== null).toBe(offered)
  })
})

describe('Creating a Team or a Project is offered to a Workspace owner or admin', () => {
  it.each([
    ['MEMBER', false],
    ['ADMIN', true],
    ['OWNER', true],
  ])('offers New team and New project to %s: %s', async (role, offered) => {
    api.listWorkspaces.mockResolvedValue(workspaces(role))
    api.getSpace.mockResolvedValue(space(false))
    const client = mount('/w/w/projects', 'projects')
    await waitFor(() => expect(client.getQueryData(queryKeys.space('w'))).toBeTruthy())
    await waitFor(() => expect(client.getQueryData(queryKeys.workspaces)).toBeTruthy())
    expect(screen.queryByRole('button', { name: 'New team' }) !== null).toBe(offered)
    expect(screen.queryByRole('button', { name: /\+ New project/ }) !== null).toBe(offered)
  })

  it('offers New team even while the Workspace has no Team, and opens the team dialog', async () => {
    api.listWorkspaces.mockResolvedValue(workspaces('OWNER'))
    api.getSpace.mockResolvedValue(space(false))
    mount('/w/w/projects', 'projects')
    fireEvent.click(await screen.findByRole('button', { name: 'New team' }))
    expect(open).toHaveBeenCalledWith('team')
  })
})
