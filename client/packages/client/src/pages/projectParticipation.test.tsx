import { cleanup, screen } from '@testing-library/react'
import { createElement } from 'react'
import { Route, Routes } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Space } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getSpace: vi.fn() }))
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

function mount(path: string) {
  return renderWithProviders(<Routes>
      <Route path="/w/:workspaceId/projects" element={<ProjectsPage />} />
      <Route path="/w/:workspaceId/projects/:projectId" element={<ProjectsPage />} />
      <Route path="/w/:workspaceId/projects/:projectId/:view" element={<ProjectsPage />} />
      </Routes>, { route: path })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'ko-KR')
  api.getSpace.mockResolvedValue(space)
})
afterEach(cleanup)

describe('Project participation', () => {
  it('lists every Project and says which ones the person takes part in', async () => {
    mount('/w/w/projects')
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
