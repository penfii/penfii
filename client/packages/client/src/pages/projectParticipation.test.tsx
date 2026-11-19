import { cleanup, fireEvent, screen, within } from '@testing-library/react'
import { createElement } from 'react'
import { Route, Routes, useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Space } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), getFavorites: vi.fn(), me: vi.fn() }))
vi.mock('../api', () => ({ api }))
vi.mock('../modules/work/Work', () => ({ Work: () => createElement('p', null, 'INSIDE') }))
vi.mock('../modules/work/PlanningPage', () => ({ PlanningPage: () => createElement('p', null, 'INSIDE') }))
vi.mock('../modules/docs/DocsScreen', () => ({ DocsScreen: () => createElement('p', null, 'INSIDE') }))
vi.mock('../modules/work/Timeline', () => ({ TimelinePage: () => createElement('p', null, 'INSIDE') }))
vi.mock('../modules/work/ReportPanels', () => ({ StatusSummary: () => createElement('p', null, 'INSIDE') }))
vi.mock('../modules/personal/PinsPanel', () => ({ PinsPanel: () => createElement('p', null, 'INSIDE') }))
vi.mock('../modules/personal/SavedButtons', () => ({ FavoriteButton: () => null }))
vi.mock('../modules/attention/WatchButton', () => ({ WatchButton: () => null }))
vi.mock('../modules/attention/ProjectSettings', () => ({ ProjectSettings: () => createElement('p', null, 'INSIDE'), SettingsLink: () => null }))
vi.mock('../shell/create', () => ({ useCreate: () => () => undefined }))
vi.mock('./SpacePage', () => ({ ConversationFrame: () => createElement('p', null, 'INSIDE') }))

import { ProjectsPage } from './ProjectsPage'
import { renderWithProviders } from '../test/renderWithProviders'

const space = {
  teams: [],
  projects: [
    { id: 'in', workspaceId: 'w', name: 'Mine', teamIds: [], participating: true },
    { id: 'out', workspaceId: 'w', name: 'Theirs', teamIds: [], participating: false },
  ],
  channels: [],
  dms: [],
} as unknown as Space

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.pathname + location.search}</output>
}

function mount(path: string) {
  return renderWithProviders(<><Where /><Routes>
      <Route path="/w/:workspaceId/projects" element={<ProjectsPage />} />
      <Route path="/w/:workspaceId/projects/:projectId" element={<ProjectsPage />} />
      <Route path="/w/:workspaceId/projects/:projectId/:view" element={<ProjectsPage />} />
      </Routes></>, { route: path })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'ko-KR')
  api.getSpace.mockResolvedValue(space)
  api.getFavorites.mockResolvedValue([])
  api.me.mockResolvedValue({ id: 'u1' })
  localStorage.removeItem('penfii.recentProjects.w.u1')
})
afterEach(cleanup)

describe('Project participation', () => {
  it('opens on the Projects the person takes part in, and finds every Project in All projects', async () => {
    mount('/w/w/projects')
    expect(await screen.findByRole('link', { name: /Mine/ })).toBeTruthy()
    expect(screen.queryByRole('link', { name: /Theirs/ })).toBeNull()
    cleanup()
    mount('/w/w/projects?browse=all')
    const mine = await screen.findByRole('link', { name: /Mine/ })
    const theirs = await screen.findByRole('link', { name: /Theirs/ })
    expect(mine.textContent).toContain('참여 중')
    expect(theirs.textContent).toContain('참여 필요')
  })

  it.each(['', '/docs', '/work', '/planning', '/settings'])('opens a Project inside only for a participant (%s)', async (tab) => {
    mount(`/w/w/projects/in${tab}`)
    expect((await screen.findAllByText('INSIDE')).length).toBeGreaterThan(0)
    expect(screen.queryByText('이 프로젝트에 참여하고 있지 않습니다.')).toBeNull()
  })

  it.each(['', '/docs', '/work', '/planning', '/settings'])('shows only the not-participating state, nothing to create, for a Project the person is not in (%s)', async (tab) => {
    mount(`/w/w/projects/out${tab}`)
    expect(await screen.findByText('이 프로젝트에 참여하고 있지 않습니다.')).toBeTruthy()
    expect(screen.queryByText('INSIDE')).toBeNull()
    expect(screen.queryByRole('button')).toBeNull()
    expect(screen.queryByText(/첫 페이지/)).toBeNull()
  })
})

const star = (targetId: string, position: number) => ({ kind: 'PROJECT', targetId, position, createdAt: '', available: true, title: null, subtitle: null, projectId: null, teamId: null, viewTarget: null, layout: null })
const many = (names: string[], extra: Record<string, unknown> = {}) => ({
  ...space,
  projects: names.map((name, index) => ({ id: `p${index}`, workspaceId: 'w', name, teamIds: [], participating: true, ...extra })),
}) as unknown as Space

describe('My projects and All projects (FEA-117)', () => {
  it('shows starred Projects first, once each, and keeps starred Projects the person is not in out of it', async () => {
    api.getSpace.mockResolvedValue(many(['Alpha', 'Beta', 'Gamma']))
    api.getFavorites.mockResolvedValue([star('p2', 1), star('p0', 2), star('missing', 3)])
    mount('/w/w/projects')
    const favorites = await screen.findByRole('region', { name: /Favorites|즐겨찾기/ })
    expect(within(favorites).getAllByRole('link').map((link) => link.textContent)).toEqual(['★Gamma', '★Alpha'])
    expect(screen.getAllByRole('link', { name: /Gamma/ })).toHaveLength(1)
    expect(screen.getByRole('link', { name: /Beta/ })).toBeTruthy()
  })

  it('explains an empty "mine" and leads to All projects when the person is in none', async () => {
    api.getSpace.mockResolvedValue({ ...space, projects: [{ id: 'out', workspaceId: 'w', name: 'Theirs', teamIds: [], participating: false }] } as unknown as Space)
    mount('/w/w/projects')
    expect(await screen.findByText('참여 중인 프로젝트가 없습니다')).toBeTruthy()
    fireEvent.click(screen.getByRole('link', { name: '모든 프로젝트 둘러보기' }))
    expect(await screen.findByRole('link', { name: /Theirs/ })).toBeTruthy()
    expect(screen.getByTestId('where').textContent).toBe('/w/w/projects?browse=all')
  })

  it('searches and filters All projects in the address, with counts, and says when nothing matches', async () => {
    api.getSpace.mockResolvedValue(space)
    mount('/w/w/projects?browse=all')
    await screen.findByRole('link', { name: /Mine/ })
    fireEvent.click(screen.getByRole('button', { name: /미참여/ }))
    expect(screen.getByTestId('where').textContent).toContain('join=out')
    expect(screen.queryByRole('link', { name: /Mine/ })).toBeNull()
    expect(screen.getByRole('link', { name: /Theirs/ }).textContent).toContain('참여 필요')
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'zzz' } })
    expect(screen.getByTestId('where').textContent).toContain('q=zzz')
    expect(await screen.findByText('조건에 맞는 프로젝트가 없습니다')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '검색과 필터 지우기' }))
    expect(await screen.findByRole('link', { name: /Mine/ })).toBeTruthy()
  })

  it('leads a person on a Project they are not in back to All projects', async () => {
    mount('/w/w/projects/out')
    fireEvent.click(await screen.findByRole('link', { name: '모든 프로젝트로 돌아가기' }))
    expect(screen.getByTestId('where').textContent).toBe('/w/w/projects?browse=all')
  })
})

describe('Switching Projects from the name (FEA-119)', () => {
  it('has no switcher with one participating Project', async () => {
    mount('/w/w/projects/in')
    await screen.findAllByText('INSIDE')
    expect(screen.getByRole('heading', { level: 1 }).querySelector('button')).toBeNull()
  })

  it('lists only Projects the person is in, finds one by search and keeps the section, dropping everything else', async () => {
    api.getSpace.mockResolvedValue(many(['Alpha', 'Beta', 'Gamma']))
    api.getFavorites.mockResolvedValue([star('p2', 1)])
    mount('/w/w/projects/p0/timeline?mode=calendar&panel=ticket:x')
    const trigger = await screen.findByRole('button', { name: /Alpha/ })
    fireEvent.click(trigger)
    const dialog = await screen.findByRole('dialog', { name: /Alpha/ })
    const names = within(dialog).getAllByRole('link').map((link) => link.textContent)
    // Favorites, then (no recent visits yet besides the current one) the rest; the current one is marked in words.
    expect(names[0]).toContain('Gamma')
    expect(within(dialog).getByRole('link', { name: /Alpha/ }).getAttribute('aria-current')).toBe('true')
    fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: 'bet' } })
    expect(within(dialog).queryByRole('link', { name: /Gamma/ })).toBeNull()
    fireEvent.click(within(dialog).getByRole('link', { name: /Beta/ }))
    expect(screen.getByTestId('where').textContent).toBe('/w/w/projects/p1/timeline')
  })

  it('says when nothing matches and offers all Projects', async () => {
    api.getSpace.mockResolvedValue(many(['Alpha', 'Beta']))
    mount('/w/w/projects/p0')
    fireEvent.click(await screen.findByRole('button', { name: /Alpha/ }))
    const dialog = await screen.findByRole('dialog')
    fireEvent.change(within(dialog).getByRole('searchbox'), { target: { value: 'nope' } })
    expect(within(dialog).getByText(/검색 결과가 없습니다/)).toBeTruthy()
    expect(within(dialog).getByRole('link', { name: '모든 프로젝트 보기' }).getAttribute('href')).toContain('browse=all&q=nope')
  })

  it('remembers real visits on this device and lists them after the favorites', async () => {
    api.getSpace.mockResolvedValue(many(['Alpha', 'Beta', 'Gamma', 'Delta']))
    api.getFavorites.mockResolvedValue([])
    mount('/w/w/projects/p2')
    await screen.findByRole('button', { name: /Gamma/ })
    cleanup()
    mount('/w/w/projects/p0')
    fireEvent.click(await screen.findByRole('button', { name: /Alpha/ }))
    const dialog = await screen.findByRole('dialog')
    const recent = within(dialog).getByRole('region', { name: /최근 방문/ })
    // Newest first: the Project open now, then the one visited before it; the current one is named in words.
    expect(within(recent).getAllByRole('link').map((link) => link.textContent)).toEqual(['Alpha현재', 'Gamma'])
  })
})
