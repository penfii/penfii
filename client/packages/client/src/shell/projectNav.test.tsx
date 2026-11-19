import { cleanup, screen, within } from '@testing-library/react'
import { Route, Routes } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), listWorkspaces: vi.fn(), getFavorites: vi.fn() }))
vi.mock('../api', () => ({ api }))
vi.mock('./create', () => ({ useCreate: () => () => undefined }))

import { ContextSidebar } from './ContextSidebar'
import { renderWithProviders } from '../test/renderWithProviders'

const star = (targetId: string, position: number) => ({ kind: 'PROJECT', targetId, position, createdAt: '', available: true, title: null, subtitle: null, projectId: null, teamId: null, viewTarget: null, layout: null })
const projects = (count: number, extra: (index: number) => Record<string, unknown> = () => ({})) =>
  Array.from({ length: count }, (_, index) => ({ id: `p${index}`, name: `Project ${String(index).padStart(2, '0')}`, teamIds: [], participating: true, ...extra(index) }))

function mount(path: string, space: Record<string, unknown>) {
  api.getSpace.mockResolvedValue({ teams: [{ id: 't1', name: 'Platform', archivedAt: null, description: null, departmentId: null }], channels: [], dms: [], ...space })
  return renderWithProviders(
    <Routes>
      <Route path="/w/:workspaceId/*" element={<ContextSidebar workspaceId="w" section="projects" />} />
    </Routes>,
    { route: path },
  )
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.listWorkspaces.mockResolvedValue([{ id: 'w', name: 'Ourspace', role: 'OWNER' }])
  api.getFavorites.mockResolvedValue([])
})
afterEach(cleanup)

describe('Projects navigation', () => {
  it('puts the shortcuts first, then only the Projects the person is in; no outside Projects and no Teams', async () => {
    mount('/w/w/projects', { projects: [...projects(2), { id: 'x', name: 'Outsider', teamIds: [], participating: false }] })
    const shortcuts = await screen.findByRole('region', { name: 'Shortcuts' })
    expect(within(shortcuts).getAllByRole('link').map((link) => link.textContent)).toEqual(['My Work', 'All projects'])
    expect(await screen.findByRole('link', { name: 'Project 00' })).toBeTruthy()
    expect(screen.queryByText('Outsider')).toBeNull()
    expect(screen.queryByText('Platform')).toBeNull()
    expect(screen.queryByRole('button', { name: 'New team' })).toBeNull()
    expect(screen.getByRole('link', { name: /My projects/ }).getAttribute('aria-current')).toBe('page')
  })

  it('marks All projects, not My projects, as the place when browsing all', async () => {
    mount('/w/w/projects?browse=all', { projects: projects(2) })
    const all = await screen.findByRole('link', { name: 'All projects' })
    expect(all.getAttribute('aria-current')).toBe('page')
    expect(screen.getByRole('link', { name: /My projects/ }).getAttribute('aria-current')).toBeNull()
  })

  it('shows every favorite and the first of the rest, and leads to the full list when there are more', async () => {
    const list = projects(24)
    api.getFavorites.mockResolvedValue(list.slice(10, 21).map((project, index) => star(project.id, index)))
    mount('/w/w/projects', { projects: list })
    await screen.findByRole('link', { name: /Project 10/ })
    // Eleven favorites are all there (more than the usual eight), none twice.
    expect(screen.getAllByRole('img', { name: 'Favorite' })).toHaveLength(11)
    expect(screen.getAllByRole('link', { name: /Project 10/ })).toHaveLength(1)
    const more = screen.getByRole('link', { name: 'All 24 Projects you take part in' })
    expect(more.getAttribute('href')).toBe('/w/w/projects')
  })

  it('says so when the person is in no Project, and still offers All projects', async () => {
    mount('/w/w/projects', { projects: projects(3, () => ({ participating: false })) })
    expect(await screen.findByText('You are not in any Project.')).toBeTruthy()
    expect(screen.getByRole('link', { name: 'All projects' })).toBeTruthy()
  })
})
