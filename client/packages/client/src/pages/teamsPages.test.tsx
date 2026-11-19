import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { createElement } from 'react'
import { Route, Routes, useLocation, useNavigate } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  getSpace: vi.fn(), listWorkspaces: vi.fn(), me: vi.fn(), listPeople: vi.fn(), listDepartments: vi.fn(),
  listTeamMembers: vi.fn(), getTeamProjects: vi.fn(),
}))
vi.mock('../api', () => ({ api }))
vi.mock('../modules/work/Work', () => ({ Work: () => createElement('p', null, 'WORK') }))
vi.mock('../modules/spaces/TeamPreview', () => ({ TeamPreview: ({ team }: { team: { name: string } }) => createElement('aside', null, `PREVIEW ${team.name}`) }))
vi.mock('../modules/spaces/TeamMembersPanel', () => ({ TeamMembersPanel: () => createElement('p', null, 'ROSTER'), TEAM_ROLES: ['MEMBER', 'MANAGER'] }))
vi.mock('../modules/spaces/TeamProjectsTab', () => ({ TeamProjectsTab: () => null }))
vi.mock('../modules/spaces/TeamChannelsTab', () => ({ TeamChannelsTab: () => null }))
vi.mock('../modules/spaces/TeamInfoForm', () => ({ TeamInfoForm: () => null }))
vi.mock('../modules/spaces/TeamArchiveSection', () => ({ TeamArchiveSection: () => null, RestoreTeamButton: () => null }))
vi.mock('../modules/spaces/OrgStructureEditor', () => ({ OrgStructureEditor: () => null }))
vi.mock('../modules/spaces/BatchAssignDialog', () => ({ BatchAssignDialog: () => null }))
vi.mock('../shell/WorkspaceShell', () => ({ OpenCreate: ({ label }: { label: string }) => createElement('button', { type: 'button' }, label) }))

import { TeamsPage } from './TeamsPage'
import { TeamsNav } from './TeamsNav'
import { renderWithProviders } from '../test/renderWithProviders'

const departments = [
  { id: 'd1', workspaceId: 'w', parentId: null, name: 'Product', position: 0 },
  { id: 'd2', workspaceId: 'w', parentId: 'd1', name: 'Dev', position: 0 },
  { id: 'd3', workspaceId: 'w', parentId: null, name: 'Sales', position: 1 },
]
const team = (id: string, name: string, departmentId: string | null, extra: Record<string, unknown> = {}) => ({ id, name, description: null, archivedAt: null, departmentId, ...extra })
const teams = [
  team('t1', 'Platform', 'd2'),
  team('t2', 'Design', 'd1'),
  team('t3', 'Design', 'd3'),
  team('t4', 'Guild', null),
  team('t5', 'Old team', 'd2', { archivedAt: '2026-01-01T00:00:00Z' }),
]
const person = (userId: string, displayName: string, email: string, teamList: Array<{ id: string; name: string }>) => ({ userId, displayName, email, state: 'ACTIVE', role: 'MEMBER', teams: teamList, jobTitle: null, statusEmoji: null, statusText: null, hasAvatar: false })
const people = [
  person('me', 'Mina Me', 'mina@example.com', [{ id: 't1', name: 'Platform' }, { id: 't3', name: 'Design' }]),
  person('ana', 'Ana Park', 'ana@example.com', [{ id: 't2', name: 'Design' }]),
  person('lone', 'Lone Wolf', 'lone@example.com', []),
]

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.pathname + location.search}</output>
}
function Back() {
  const navigate = useNavigate()
  return <button type="button" onClick={() => void navigate(-1)}>history back</button>
}

function mount(path: string) {
  return renderWithProviders(
    <>
      <Where />
      <Back />
      <Routes>
        <Route path="/w/:workspaceId/directory" element={<TeamsPage />} />
        <Route path="/w/:workspaceId/directory/teams" element={<TeamsPage />} />
        <Route path="/w/:workspaceId/directory/members" element={<TeamsPage />} />
        <Route path="/w/:workspaceId/directory/teams/:teamId" element={<TeamsPage />} />
      </Routes>
    </>,
    { route: path },
  )
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  sessionStorage.clear()
  HTMLDialogElement.prototype.showModal ??= function showModal(this: HTMLDialogElement) { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function close(this: HTMLDialogElement) { this.removeAttribute('open') }
  Element.prototype.scrollIntoView = vi.fn()
  api.getSpace.mockResolvedValue({ teams, projects: [], channels: [], dms: [] })
  api.listWorkspaces.mockResolvedValue([{ id: 'w', name: 'Ourspace', role: 'OWNER' }])
  api.me.mockResolvedValue({ id: 'me' })
  api.listPeople.mockResolvedValue(people)
  api.listDepartments.mockResolvedValue(departments)
  api.listTeamMembers.mockResolvedValue([])
  api.getTeamProjects.mockResolvedValue([])
})
afterEach(cleanup)

describe('Teams navigation (FEA-120)', () => {
  it('fixes the org chart, all Teams and all members, then lists the Teams the person is on without repeats', async () => {
    renderWithProviders(<TeamsNav workspaceId="w" />, { route: '/w/w/directory/teams' })
    const links = async () => (await screen.findAllByRole('link')).map((link) => link.textContent)
    await waitFor(async () => expect(await links()).toEqual(['Organization', 'All teams', 'All members', 'Design', 'Platform']))
    expect(screen.queryByText(/Members not on a team/)).toBeNull()
    expect(screen.getByRole('link', { name: 'All teams' }).getAttribute('aria-current')).toBe('page')
    expect(screen.getByRole('link', { name: 'Organization' }).getAttribute('aria-current')).toBeNull()
  })

  it('says so when the person is on no Team', async () => {
    api.listPeople.mockResolvedValue([person('me', 'Mina Me', 'mina@example.com', [])])
    renderWithProviders(<TeamsNav workspaceId="w" />, { route: '/w/w/directory' })
    expect(await screen.findByText(/You are not on a team/)).toBeTruthy()
    expect(screen.getByRole('link', { name: 'All teams' })).toBeTruthy()
  })
})

describe('All teams (FEA-120)', () => {
  it('lists the visible Teams with their department path, so same-named Teams can be told apart', async () => {
    mount('/w/w/directory/teams')
    const rows = await screen.findAllByRole('listitem')
    const text = rows.map((row) => row.textContent ?? '')
    expect(text.filter((line) => line.startsWith('Design')).map((line) => (line.includes('Product') ? 'product' : line.includes('Sales') ? 'sales' : '?')).sort()).toEqual(['product', 'sales'])
    expect(text.some((line) => line.includes('Product › Dev'))).toBe(true)
    expect(text.some((line) => line.startsWith('Guild') && line.includes('No department'))).toBe(true)
    // Archived Teams stay out until asked for; the way into one Team is its detail.
    expect(screen.queryByText('Old team')).toBeNull()
    expect(screen.getAllByRole('link', { name: 'Platform' })[0]?.getAttribute('href')).toBe('/w/w/directory/teams/t1')
  })

  it('searches by department in the address and clears the search', async () => {
    mount('/w/w/directory/teams')
    await screen.findByRole('link', { name: 'Guild' })
    fireEvent.change(screen.getByLabelText('Search teams by name or department'), { target: { value: 'sales' } })
    expect(screen.getByTestId('where').textContent).toContain('q=sales')
    expect(screen.getAllByRole('link', { name: 'Design' })).toHaveLength(1)
    expect(screen.queryByRole('link', { name: 'Platform' })).toBeNull()
    fireEvent.change(screen.getByLabelText('Search teams by name or department'), { target: { value: 'zzz' } })
    fireEvent.click(await screen.findByRole('button', { name: 'Clear search and scope' }))
    expect(await screen.findByRole('link', { name: 'Guild' })).toBeTruthy()
    expect(screen.getByTestId('where').textContent).toBe('/w/w/directory/teams')
  })

  it('filters My teams and No department', async () => {
    mount('/w/w/directory/teams')
    await screen.findByRole('link', { name: 'Guild' })
    fireEvent.click(screen.getByRole('button', { name: 'My teams' }))
    expect(screen.getByTestId('where').textContent).toContain('filter=mine')
    expect(screen.getAllByRole('link', { name: /^(Platform|Design)$/ })).toHaveLength(2)
    expect(screen.queryByRole('link', { name: 'Guild' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'No department' }))
    expect(await screen.findByRole('link', { name: 'Guild' })).toBeTruthy()
    expect(screen.queryByRole('link', { name: 'Platform' })).toBeNull()
  })

  it('offers New team here, to a Workspace manager', async () => {
    mount('/w/w/directory/teams')
    expect(await screen.findByRole('button', { name: '+ New team' })).toBeTruthy()
  })
})

describe('Finding a Team in the org chart (FEA-121)', () => {
  it('highlights the Team in words, opens its preview and ends the location on request', async () => {
    mount('/w/w/directory?focus=team:t1')
    expect(await screen.findByText('Found team')).toBeTruthy()
    expect(screen.getByText('PREVIEW Platform')).toBeTruthy()
    expect(screen.getByText('Team found in the org chart: Platform')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'End location' }))
    expect(screen.getByTestId('where').textContent).toBe('/w/w/directory')
    await waitFor(() => expect(screen.queryByText('Found team')).toBeNull())
  })

  it('shows an archived Team that the archive setting would hide, without changing that setting', async () => {
    mount('/w/w/directory?focus=team:t5')
    expect(await screen.findByText('Found team')).toBeTruthy()
    expect(screen.getByText(/lifted for this screen only/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'End location' }))
    await waitFor(() => expect(screen.queryByText('Old team')).toBeNull())
  })

  it('opens safely, saying nothing about a Team that is gone or cannot be seen', async () => {
    mount('/w/w/directory?focus=team:nope')
    expect(await screen.findByText(/cannot be found, or you may not see it/)).toBeTruthy()
    expect(screen.getByRole('heading', { name: 'Organization' })).toBeTruthy()
  })

  it('shows the department path and a link to the org chart in the Team detail', async () => {
    mount('/w/w/directory/teams/t1')
    expect(await screen.findByText('Department: Product › Dev')).toBeTruthy()
    fireEvent.click(screen.getByRole('link', { name: 'View in org chart' }))
    expect(screen.getByTestId('where').textContent).toBe('/w/w/directory?focus=team:t1')
    expect(await screen.findByText('Found team')).toBeTruthy()
  })
})

describe('Member search and Back (FEA-121)', () => {
  it('returns to the typed search, the filter and the person after opening a Team, and keeps the text out of the address', async () => {
    mount('/w/w/directory/members')
    await screen.findByRole('button', { name: /Ana Park/ })
    fireEvent.change(screen.getByLabelText('Search name or email'), { target: { value: 'ana@' } })
    expect(screen.queryByText('Lone Wolf')).toBeNull()
    expect(screen.getByTestId('where').textContent).toBe('/w/w/directory/members')
    // The row shows the Team with its department so same-named Teams differ.
    expect(document.querySelector('.person-team')?.textContent).toContain('Design · Product')
    fireEvent.click(screen.getByRole('button', { name: /Ana Park/ }))
    const card = await screen.findByRole('dialog')
    fireEvent.click(within(card).getByRole('link', { name: 'Open team Design' }))
    expect(screen.getByTestId('where').textContent).toBe('/w/w/directory/teams/t2')
    fireEvent.click(screen.getByRole('button', { name: 'history back' }))
    await screen.findByRole('button', { name: /Ana Park/ })
    expect((screen.getByLabelText('Search name or email') as HTMLInputElement).value).toBe('ana@')
    expect(screen.queryByText('Lone Wolf')).toBeNull()
    expect(screen.getByTestId('where').textContent).toBe('/w/w/directory/members')
  })

  it('offers each Team of a person with its own org chart link, and says so for a person on no Team', async () => {
    mount('/w/w/directory/members')
    fireEvent.click(await screen.findByRole('button', { name: /Mina Me/ }))
    const card = await screen.findByRole('dialog')
    expect(within(card).getAllByRole('link', { name: /View team .* in the org chart/ })).toHaveLength(2)
    fireEvent.click(within(card).getByRole('link', { name: 'View team Design in the org chart' }))
    expect(screen.getByTestId('where').textContent).toBe('/w/w/directory?focus=team:t3')
    cleanup()
    mount('/w/w/directory/members')
    expect(await screen.findAllByText('No team')).toBeTruthy()
  })

  it('does not carry a search into a fresh visit, a new tab or another Workspace', async () => {
    const first = mount('/w/w/directory/members')
    fireEvent.change(await screen.findByLabelText('Search name or email'), { target: { value: 'ana' } })
    first.unmount()
    // A fresh visit (new navigation, other history entry) starts empty, as does another Workspace.
    sessionStorage.clear()
    mount('/w/other/directory/members')
    expect((await screen.findByLabelText('Search name or email') as HTMLInputElement).value).toBe('')
  })
})
