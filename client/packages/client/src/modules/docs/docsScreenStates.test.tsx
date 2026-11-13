import { cleanup, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getDocTree: vi.fn(), getDocTemplates: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { DocsScreen } from './DocsScreen'
import { renderWithProviders } from '../../test/renderWithProviders'

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getDocTemplates.mockResolvedValue([])
})
afterEach(cleanup)

const mount = () => renderWithProviders(<DocsScreen withTree />, { route: '/w/w/docs', path: '/w/:workspaceId/docs' })

describe('The Docs screen without a page open', () => {
  it('invites to create the first page only when the list loaded and is empty', async () => {
    api.getDocTree.mockResolvedValue([])
    mount()
    // Both the tree and the empty main pane say it.
    expect((await screen.findAllByText('Create the first page')).length).toBeGreaterThan(0)
  })

  it('does not claim there are no pages while the list is loading or failed to load', async () => {
    api.getDocTree.mockRejectedValue(new Error('boom'))
    mount()
    expect((await screen.findAllByRole('alert')).length).toBeGreaterThan(0)
    expect(screen.queryAllByText('Create the first page')).toHaveLength(0)
    expect(screen.getByText('Choose a document from the list or create a new one')).toBeTruthy()
  })
})
