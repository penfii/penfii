import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  listTeamMembers: vi.fn(),
  listPeople: vi.fn(),
  searchDirectory: vi.fn(),
  placeTeam: vi.fn(),
  updateDepartment: vi.fn(),
  listDepartments: vi.fn(),
  getSpace: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { OrgStructureEditor } from './OrgStructureEditor'
import { TeamMembersPanel } from './TeamMembersPanel'
import type { OrgMove } from './orgTree'
import { useState } from 'react'
import { renderWithProviders } from '../../test/renderWithProviders'

const departments = [
  { id: 'd1', workspaceId: 'w', parentId: null, name: 'Product', position: 0 },
  { id: 'd2', workspaceId: 'w', parentId: null, name: 'Design', position: 1 },
]
const teams = [{ id: 't1', name: 'Frontend', archivedAt: null, description: null, departmentId: 'd1' }]

function wrap(node: React.ReactNode) {
  return renderWithProviders(node, { route: '/' })
}

function Editor() {
  const [moves, setMoves] = useState<OrgMove[]>([])
  return <OrgStructureEditor workspaceId="w" departments={departments} teams={teams} moves={moves} onMoves={setMoves} />
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  for (const fn of Object.values(api)) fn.mockReset()
  api.searchDirectory.mockResolvedValue({ query: '', results: [] })
  HTMLDialogElement.prototype.showModal ??= function showModal(this: HTMLDialogElement) { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function close(this: HTMLDialogElement) { this.removeAttribute('open') }
})
afterEach(cleanup)

describe('Staged organization editing', () => {
  it('keeps a move staged until it is applied, and can undo it', async () => {
    api.placeTeam.mockResolvedValue({})
    api.listDepartments.mockResolvedValue(departments)
    api.getSpace.mockResolvedValue({ teams, projects: [], channels: [] })
    wrap(<Editor />)
    fireEvent.change(screen.getByLabelText('Frontend: department'), { target: { value: 'd2' } })
    expect(await screen.findByText('1 changes')).toBeTruthy()
    expect(screen.getByText('Frontend: Product → Design')).toBeTruthy()
    expect(api.placeTeam).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }))
    expect(screen.queryByText('1 changes')).toBeNull()
    fireEvent.change(screen.getByLabelText('Frontend: department'), { target: { value: 'd2' } })
    fireEvent.click(await screen.findByRole('button', { name: 'Apply changes' }))
    await waitFor(() => expect(api.placeTeam).toHaveBeenCalledWith('w', 't1', 'd2'))
    expect(await screen.findByText('Changes applied.')).toBeTruthy()
  })

  it('refuses to put a department inside itself without asking the server', async () => {
    wrap(<Editor />)
    // The parent choices of a department leave out itself and everything below it.
    const options = Array.from((screen.getByLabelText('Product: parent department') as HTMLSelectElement).options).map((option) => option.text)
    expect(options).toEqual(['Top level', 'Design'])
  })
})

describe('Team roster for a member who does not run the Team', () => {
  const roster = [
    { userId: 'u1', displayName: 'Ann', email: 'ann@x', role: 'MANAGER' },
    { userId: 'u2', displayName: 'Bob', email: 'bob@x', role: 'MEMBER' },
  ]
  const people = [
    { userId: 'u1', displayName: 'Ann', email: 'ann@x', hasAvatar: false, role: 'OWNER', state: 'ACTIVE', teams: [{ id: 't1', name: 'Core' }, { id: 't2', name: 'Growth' }] },
    { userId: 'u2', displayName: 'Bob', email: 'bob@x', hasAvatar: false, role: 'MEMBER', state: 'ACTIVE', teams: [{ id: 't1', name: 'Core' }] },
  ]
  beforeEach(() => {
    api.listTeamMembers.mockResolvedValue(roster)
    api.listPeople.mockResolvedValue(people)
  })

  it('reads the roster with other Teams but has no role select, remove or picker, and no Workspace role', async () => {
    wrap(<TeamMembersPanel workspaceId="w" teamId="t1" canManage={false} />)
    expect((await screen.findAllByText('Ann')).length).toBeGreaterThan(0)
    expect((await screen.findAllByText('Growth')).length).toBeGreaterThan(0)
    expect(screen.getAllByText('Team manager').length).toBeGreaterThan(0)
    expect(screen.queryByLabelText('Ann: Role')).toBeNull()
    expect(screen.queryByLabelText('Remove Ann')).toBeNull()
    expect(screen.queryByText('Workspace role')).toBeNull()
    expect(screen.queryByText(/Add members/)).toBeNull()
  })

  it('shows the Workspace role to owners and admins, and filters by name or email', async () => {
    wrap(<TeamMembersPanel workspaceId="w" teamId="t1" canManage showWorkspaceRole />)
    expect(await screen.findByText('Workspace role')).toBeTruthy()
    expect(await screen.findByText('Owner')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Search name or email'), { target: { value: 'bob@' } })
    expect(screen.queryByLabelText('Select Ann')).toBeNull()
    expect(screen.getByLabelText('Select Bob')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Search name or email'), { target: { value: 'zzz' } })
    expect(await screen.findByText('No member matches.')).toBeTruthy()
  })
})
