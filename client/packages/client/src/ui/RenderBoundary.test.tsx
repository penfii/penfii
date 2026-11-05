import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../i18n/I18nProvider'
import { RenderBoundary } from './RenderBoundary'

function Broken(): never {
  throw new Error('chunk did not load')
}

function mount(resetKey: string, broken: boolean) {
  return (
    <I18nProvider>
      <nav>navigation</nav>
      <RenderBoundary resetKey={resetKey}>{broken ? <Broken /> : <p>screen</p>}</RenderBoundary>
    </I18nProvider>
  )
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  vi.spyOn(console, 'error').mockImplementation(() => undefined)
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('RenderBoundary', () => {
  it('keeps what is around a failing screen and offers a reload', () => {
    render(mount('/a', true))
    expect(screen.getByText('navigation')).toBeTruthy()
    expect(screen.getByRole('alert').textContent).toContain('This screen could not be shown')
    expect(screen.getByRole('button', { name: 'Reload' })).toBeTruthy()
  })

  it('tries again when the person goes to another screen', () => {
    const { rerender } = render(mount('/a', true))
    expect(screen.getByRole('alert')).toBeTruthy()
    rerender(mount('/b', false))
    expect(screen.getByText('screen')).toBeTruthy()
    expect(screen.queryByRole('alert')).toBeNull()
  })
})
