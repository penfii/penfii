import { cleanup, fireEvent, screen } from '@testing-library/react'
import { useLocation } from 'react-router'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { SearchRoot, searchRootFrom } from './RootScreens'
import { SearchNav } from '../shell/nav/SectionNavs'
import { renderWithProviders } from '../test/renderWithProviders'

function Where() {
  const { pathname, search } = useLocation()
  return <output data-testid="where">{decodeURIComponent(pathname + search)}</output>
}

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(cleanup)

describe('FEA-111: changing the kind keeps the search', () => {
  it('desktop: every kind link carries the text and the Project state', () => {
    renderWithProviders(<SearchNav workspaceId="w" />, { route: '/w/w/search?tab=ticket&q=FEA-1%20project%3AFEA&state=s1' })
    for (const link of screen.getAllByRole('link')) {
      const href = decodeURIComponent(link.getAttribute('href') ?? '')
      expect(href).toContain('q=FEA-1 project:FEA')
      expect(href).toContain('state=s1')
    }
  })

  it('phone: going back from results lands on a list that starts with the same search', () => {
    expect(searchRootFrom('w', '?tab=ticket&q=FEA-1+project%3AFEA&state=s1')).toBe('/w/w/search?tab=ticket&text=FEA-1+project%3AFEA&state=s1')
    expect(searchRootFrom('w', '')).toBe('/w/w/search')
  })

  it('phone: the list starts with that search, and another kind searches the same text and state', () => {
    renderWithProviders(<><SearchRoot workspaceId="w" /><Where /></>, { route: '/w/w/search?tab=ticket&text=FEA-1%20project%3AFEA&state=s1' })
    const input = screen.getByRole('searchbox') as HTMLInputElement
    expect(input.value).toBe('FEA-1 project:FEA')
    fireEvent.click(screen.getByRole('button', { name: 'All' }))
    fireEvent.submit(input.closest('form')!)
    const where = screen.getByTestId('where').textContent ?? ''
    expect(where).toContain('tab=all')
    expect(where).toContain('q=FEA-1 project:FEA')
    expect(where).toContain('state=s1')
  })
})
