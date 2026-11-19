import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ search: vi.fn(), getSpace: vi.fn(), getProjectWorkflow: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { SearchPage } from './SearchPage'
import { renderWithProviders } from '../../test/renderWithProviders'

function Where() {
  const { search } = useLocation()
  return <output data-testid="where">{search}</output>
}
const mount = (route: string) =>
  renderWithProviders(<><SearchPage /><Where /></>, { route, path: '/w/:workspaceId/search' })
const url = () => decodeURIComponent(screen.getByTestId('where').textContent ?? '')

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p', name: 'Penfii App', ticketPrefix: 'PFI', teamIds: [], participating: true }], channels: [], dms: [] })
  api.getProjectWorkflow.mockResolvedValue({ states: [{ id: 's1', name: 'Doing' }] })
  api.search.mockResolvedValue({ results: [], nextCursor: null, status: { ready: true, stale: false, unavailableKinds: [] } })
})
afterEach(cleanup)

describe('Search with nothing found', () => {
  it('says which conditions applied and offers the ways out', async () => {
    mount('/w/w/search?q=FEA-1%20project%3APFI%20assignee%3Ame&tab=ticket&state=s1')
    expect(await screen.findByText('Active conditions')).toBeTruthy()
    for (const text of ['project:PFI', 'assignee:me', 'Tab: ', 'State: Doing']) expect(await screen.findByText(new RegExp(text))).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Remove project filter' }))
    await waitFor(() => expect(url()).not.toContain('project'))
    expect(url()).not.toContain('state=')
    expect(url()).toContain('FEA-1')
  })

  it('clears every condition, or searches the All tab, each into the right address', async () => {
    mount('/w/w/search?q=FEA-1%20project%3APFI&tab=ticket')
    fireEvent.click(await screen.findByRole('button', { name: 'Search everything' }))
    await waitFor(() => expect(url()).toContain('tab=all'))
    expect(url()).toContain('project:PFI')
    fireEvent.click(await screen.findByRole('button', { name: 'Clear all conditions' }))
    await waitFor(() => expect(url()).not.toContain('q='))
  })

  it('says what the project filter covers, and does not show the empty-state help while searching', async () => {
    api.search.mockReturnValue(new Promise(() => undefined))
    mount('/w/w/search?q=x%20project%3APFI')
    expect(await screen.findByText(/The project filter applies to messages, tickets, documents, spaces and views/)).toBeTruthy()
    expect(screen.queryByText('Active conditions')).toBeNull()
  })
})
