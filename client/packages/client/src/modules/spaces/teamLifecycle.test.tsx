import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  listTeamProjects: vi.fn(),
  listTeamMembers: vi.fn(),
  addTeamMember: vi.fn(),
  removeTeamMember: vi.fn(),
  searchDirectory: vi.fn(),
  listAudit: vi.fn(),
  listMembers: vi.fn(),
  getSpace: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { ShortcutHelpProvider, useShortcutHelp } from '../help/ShortcutHelp'
import { TeamMembersPanel } from './TeamMembersPanel'
import { TeamProjectsTab } from './TeamProjectsTab'
import { activeTeams, isArchivedTeam } from './teamState'
import { renderWithProviders } from '../../test/renderWithProviders'

function wrap(node: React.ReactNode) {
  return renderWithProviders(node, { route: '/' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  for (const fn of Object.values(api)) fn.mockReset()
  HTMLDialogElement.prototype.showModal ??= function showModal(this: HTMLDialogElement) { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function close(this: HTMLDialogElement) { this.removeAttribute('open') }
})
afterEach(cleanup)

describe('team state', () => {
  it('treats a Team with archivedAt as archived and leaves it out of default lists', () => {
    const teams = [{ id: 'a', name: 'A', archivedAt: null }, { id: 'b', name: 'B', archivedAt: '2026-11-03T00:00:00Z' }]
    expect(isArchivedTeam(teams[1])).toBe(true)
    expect(activeTeams(teams).map((team) => team.id)).toEqual(['a'])
  })
})

describe('Team Projects tab', () => {
  it('shows the Team role and only the viewer\'s own access, and never lists other members', async () => {
    api.listTeamProjects.mockResolvedValue([
      { projectId: 'p1', name: 'Payment', ticketPrefix: 'PAY', archivedAt: null, teamRole: 'EDITOR', canOpen: true, myRole: 'EDITOR', mySources: ['TEAM'] },
      { projectId: 'p2', name: 'Platform', ticketPrefix: 'PLT', archivedAt: null, teamRole: null, canOpen: true, myRole: null, mySources: [] },
      { projectId: 'p3', name: 'Labs', ticketPrefix: 'LAB', archivedAt: null, teamRole: 'VIEWER', canOpen: false, myRole: null, mySources: [] },
    ])
    wrap(<TeamProjectsTab workspaceId="w1" teamId="t1" />)
    expect(await screen.findByText('Me: Editor (through this Team)')).toBeTruthy()
    expect(screen.getByText('Team role: Editor')).toBeTruthy()
    expect(screen.getByText('Linked only')).toBeTruthy()
    expect(screen.getByText(/no access/)).toBeTruthy()
    expect(screen.getByText(/only its name and the Team role show/)).toBeTruthy()
    // A Project the viewer cannot open is a name, not a link.
    expect(screen.getByText('Labs').closest('a')).toBeNull()
    expect(screen.getByText('Payment').closest('a')).not.toBeNull()
  })

  it('says so when nothing is linked', async () => {
    api.listTeamProjects.mockResolvedValue([])
    wrap(<TeamProjectsTab workspaceId="w1" teamId="t1" />)
    expect(await screen.findByText(/No linked Projects/)).toBeTruthy()
  })
})

describe('Team members of an archived Team', () => {
  const roster = [
    { userId: 'u1', displayName: 'Ann', email: 'a@x', role: 'MANAGER' },
    { userId: 'u2', displayName: 'Bob', email: 'b@x', role: 'MEMBER' },
  ]
  it('cannot be changed: no picker, no select or remove', async () => {
    api.listTeamMembers.mockResolvedValue(roster)
    wrap(<TeamMembersPanel workspaceId="w1" teamId="t1" archived canManage />)
    expect((await screen.findAllByText('Ann')).length).toBeGreaterThan(0)
    expect(screen.getByText(/archived Team's members cannot be changed/)).toBeTruthy()
    expect(screen.queryByLabelText('Ann: Role')).toBeNull()
    expect(screen.queryByLabelText('Remove Ann')).toBeNull()
    expect(screen.getByText('Team manager')).toBeTruthy()
    expect(screen.queryByLabelText(/Select Ann/)).toBeNull()
  })

  it('refuses to start a batch removal that would take out the last manager', async () => {
    api.listTeamMembers.mockResolvedValue(roster)
    wrap(<TeamMembersPanel workspaceId="w1" teamId="t1" canManage />)
    fireEvent.click(await screen.findByLabelText('Select Ann'))
    fireEvent.click(screen.getByLabelText('Select Bob'))
    fireEvent.click(screen.getByRole('button', { name: 'Remove from Team' }))
    expect(await screen.findByText(/Name another manager first/)).toBeTruthy()
    expect(api.removeTeamMember).not.toHaveBeenCalled()
  })
})

describe('shortcut help', () => {
  function Bar() {
    const open = useShortcutHelp()
    return (
      <>
        <button type="button" onClick={() => open('editor')}>help</button>
        <input aria-label="field" />
      </>
    )
  }
  it('opens on ? outside text fields, never inside one, and from the editor bar at the Editor section', async () => {
    wrap(<ShortcutHelpProvider><Bar /></ShortcutHelpProvider>)
    const dialog = () => document.querySelector('dialog') as HTMLDialogElement
    expect(dialog().open).toBe(false)
    fireEvent.keyDown(screen.getByLabelText('field'), { key: '?' })
    expect(dialog().open).toBe(false)
    fireEvent.keyDown(document.body, { key: '?' })
    await waitFor(() => expect(dialog().open).toBe(true))
    expect(screen.getByRole('heading', { name: 'Editor' })).toBeTruthy()
    cleanup()
    wrap(<ShortcutHelpProvider><Bar /></ShortcutHelpProvider>)
    fireEvent.click(screen.getByText('help'))
    await waitFor(() => expect(document.activeElement?.textContent).toBe('Editor'))
  })
})
