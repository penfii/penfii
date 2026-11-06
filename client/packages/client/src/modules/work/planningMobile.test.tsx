import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fixtureBoard } from './testFixtures'

// Calls this test does not care about answer nothing, so a panel that asks for them stays empty.
const api = vi.hoisted(() => {
  const calls: Record<string, ReturnType<typeof vi.fn>> = {}
  return new Proxy(calls, { get: (target, name: string) => (target[name] ??= vi.fn().mockResolvedValue(undefined)) })
})
vi.mock('../../api', () => ({ api }))

import { PlanningPage } from './PlanningPage'
import { renderWithProviders } from '../../test/renderWithProviders'
import { DESKTOP, PHONE, setViewport } from '../../test/viewport'

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.search}</output>
}

const sprint = (id: string, name: string, state: string) => ({ id, projectId: 'p', name, goal: null, state, startsOn: '2026-11-02', endsOn: '2026-11-13', version: 1, startedAt: null, endedAt: null, timeZone: null, estimateUnit: null })

function mount(search = '') {
  return renderWithProviders(<><PlanningPage workspaceId="w" projectId="p" /><Where /></>, { route: `/w/w/projects/p/planning${search}`, path: '/w/:workspaceId/projects/:projectId/planning' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [sprint('s1', 'Sprint One', 'COMPLETED'), sprint('s2', 'Sprint Two', 'CANCELED')], milestones: [] })
  api.getProjectBoard.mockResolvedValue(fixtureBoard([]))
  api.getSprintResults.mockResolvedValue({ sprint: sprint('s1', 'Sprint One', 'COMPLETED'), baselineCount: 0, baselineRecordedAt: null, results: [] })
  api.getFavorites.mockResolvedValue([])
  api.getLater.mockResolvedValue([])
})
afterEach(cleanup)

describe('Planning on a phone', () => {
  beforeEach(() => setViewport(PHONE))

  it('opens a Sprint as its own screen: a pushed address, no Sprint/Milestone tabs, one Sprint shown', async () => {
    mount()
    expect(await screen.findByText('Sprint Two')).toBeTruthy()
    expect(screen.getByRole('tab', { name: 'Sprints' })).toBeTruthy()
    fireEvent.click(screen.getAllByRole('button', { name: 'Show tickets' })[0])
    await waitFor(() => expect(screen.getByTestId('where').textContent).toBe('?sprintDetail=s1'))
    expect(screen.queryByText('Sprint Two')).toBeNull()
    expect(screen.getByText('Sprint One')).toBeTruthy()
    expect(screen.queryByRole('tab', { name: 'Milestones' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Show tickets' })).toBeNull()
  })

  it('opens straight into a Sprint from its address', async () => {
    mount('?sprintDetail=s2')
    expect(await screen.findByText('Sprint Two')).toBeTruthy()
    expect(screen.queryByText('Sprint One')).toBeNull()
  })

  it('shows the normal list, with its tabs, for a Sprint id that is not this Project\'s', async () => {
    mount('?sprintDetail=nope')
    expect(await screen.findByText('Sprint One')).toBeTruthy()
    expect(screen.getByText('Sprint Two')).toBeTruthy()
    expect(screen.getByRole('tab', { name: 'Milestones' })).toBeTruthy()
    expect(screen.getAllByRole('button', { name: 'Show tickets' })).toHaveLength(2)
  })

  it('leaves the Sprint screen as soon as the Sprint is no longer one of the Project\'s', async () => {
    const view = mount('?sprintDetail=s2')
    expect(await screen.findByText('Sprint Two')).toBeTruthy()
    expect(screen.queryByRole('tab', { name: 'Sprints' })).toBeNull()
    api.getProjectPlanning.mockResolvedValue({ canManage: true, sprints: [sprint('s1', 'Sprint One', 'COMPLETED')], milestones: [] })
    await view.client.invalidateQueries()
    expect(await screen.findByRole('tab', { name: 'Sprints' })).toBeTruthy()
    expect(screen.getByText('Sprint One')).toBeTruthy()
  })
})

describe('Planning on a desktop-sized window', () => {
  beforeEach(() => setViewport(DESKTOP))

  it('expands a Sprint inside the list and leaves the address alone', async () => {
    mount()
    expect(await screen.findByText('Sprint Two')).toBeTruthy()
    fireEvent.click(screen.getAllByRole('button', { name: 'Show tickets' })[0])
    expect(await screen.findByRole('button', { name: 'Hide tickets' })).toBeTruthy()
    expect(screen.getByText('Sprint Two')).toBeTruthy()
    expect(screen.getByTestId('where').textContent).toBe('')
    expect(screen.getByRole('tab', { name: 'Milestones' })).toBeTruthy()
  })
})
