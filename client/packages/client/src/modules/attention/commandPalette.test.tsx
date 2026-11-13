import { cleanup, fireEvent, screen } from '@testing-library/react'
import { Route, Routes, useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ search: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { CommandPalette } from './CommandPalette'
import { renderWithProviders } from '../../test/renderWithProviders'

function Where() {
  const location = useLocation()
  return <output data-testid="where">{location.pathname}</output>
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.search.mockResolvedValue({ results: [] })
})
afterEach(cleanup)

async function openPalette() {
  renderWithProviders(
    <Routes>
      <Route path="/w/:workspaceId" element={<><CommandPalette workspaceId="w" /><Where /></>} />
      <Route path="*" element={<Where />} />
    </Routes>,
    { route: '/w/w' },
  )
  fireEvent.keyDown(document, { key: 'k', ctrlKey: true })
  return screen.findByRole('combobox')
}

describe('Command palette and input methods', () => {
  it('does not go to the highlighted place on the Enter that picks an IME candidate', async () => {
    const input = await openPalette()
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 229 })
    expect(screen.getByTestId('where').textContent).toBe('/w/w')
  })

  it('goes to the highlighted place on an ordinary Enter', async () => {
    const input = await openPalette()
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 })
    expect(screen.getByTestId('where').textContent).toBe('/w/w/inbox')
  })
})
