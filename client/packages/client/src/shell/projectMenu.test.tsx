import { cleanup, fireEvent, screen, within } from '@testing-library/react'
import { useNavigate } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getSpace: vi.fn() }))
vi.mock('../api', () => ({ api }))
vi.mock('./create', () => ({ useCreate: () => () => undefined }))

import { ContextSidebar } from './ContextSidebar'
import { renderWithProviders } from '../test/renderWithProviders'

function Harness() {
  const go = useNavigate()
  return (
    <>
      <ContextSidebar workspaceId="w" section="projects" />
      <button type="button" onClick={() => void go('/w/w/projects/p1/timeline')}>go timeline</button>
    </>
  )
}

function mount(path: string) {
  return renderWithProviders(<Harness />, { route: path, path: '/w/:workspaceId/projects/:projectId/*' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockResolvedValue({ teams: [], projects: [{ id: 'p1', name: 'Penfii App', teamIds: [] }], channels: [], dms: [] })
})
afterEach(cleanup)

describe('Project menu on narrow screens', () => {
  it('names the current section on one button and keeps every section reachable', async () => {
    mount('/w/w/projects/p1/work')
    const toggle = await screen.findByRole('button', { name: 'Project menu: Work' })
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    const sections = ['Overview', 'Work', 'Planning', 'Timeline', 'Docs', 'Chat']
    for (const label of sections) expect(screen.getByRole('link', { name: label })).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Work' }).getAttribute('aria-current')).toBe('page')
  })

  it('opens and closes by button, and closes after a section is chosen', async () => {
    mount('/w/w/projects/p1/work')
    const toggle = await screen.findByRole('button', { name: 'Project menu: Work' })
    fireEvent.click(toggle)
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
    expect(document.querySelector('.workspace-nav')?.classList.contains('menu-open')).toBe(true)
    fireEvent.click(toggle)
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    fireEvent.click(toggle)
    fireEvent.click(screen.getByText('go timeline'))
    const next = await screen.findByRole('button', { name: 'Project menu: Timeline' })
    expect(next.getAttribute('aria-expanded')).toBe('false')
    expect(within(document.querySelector('.workspace-nav') as HTMLElement).getByRole('link', { name: 'Timeline' }).getAttribute('aria-current')).toBe('page')
  })

  it('shows Overview when no section is in the address', async () => {
    mount('/w/w/projects/p1')
    expect(await screen.findByRole('button', { name: 'Project menu: Overview' })).toBeTruthy()
  })
})
