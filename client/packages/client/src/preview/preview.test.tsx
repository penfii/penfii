import { cleanup, screen } from '@testing-library/react'
import type { ReactElement } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), getDocTree: vi.fn(), getDocTemplates: vi.fn() }))
vi.mock('../api', () => ({ api }))

import { DocsScreen } from '../modules/docs/DocsScreen'
import { previewContent } from './enabled'
import { previewDocs } from './catalog'
import { shellProjects, tasksFor } from './shellModel'
import { renderWithProviders } from '../test/renderWithProviders'

function mount(ui: ReactElement) {
  return renderWithProviders(ui, { route: '/' })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
  api.getDocTree.mockResolvedValue([])
  api.getDocTemplates.mockResolvedValue([])
})
afterEach(cleanup)

describe('a real workspace has no sample content', () => {
  it('ships no sample Projects, tasks or documents unless a demo build asks', () => {
    expect(previewContent).toBe(false)
    expect(shellProjects).toEqual([])
    expect(tasksFor('penfii')).toEqual([])
    expect(previewDocs).toEqual([])
  })

  it('says so plainly instead of showing invented data', async () => {
    mount(<DocsScreen />)
    expect((await screen.findAllByText('Create the first page')).length).toBeGreaterThan(0)
  })
})
