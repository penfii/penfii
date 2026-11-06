import { act, cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { MOBILE_MAX_WIDTH, useIsMobile } from './useIsMobile'

function Probe() {
  return <i data-phone={String(useIsMobile())} />
}

const original = Object.getOwnPropertyDescriptor(window, 'matchMedia')
afterEach(() => {
  cleanup()
  if (original) Object.defineProperty(window, 'matchMedia', original)
  else Reflect.deleteProperty(window, 'matchMedia')
})

function mockMedia(matches: boolean) {
  const listeners = new Set<() => void>()
  const list = {
    matches,
    addEventListener: vi.fn((_type: string, listener: () => void) => listeners.add(listener)),
    removeEventListener: vi.fn((_type: string, listener: () => void) => listeners.delete(listener)),
  }
  const matchMedia = vi.fn(() => list)
  Object.defineProperty(window, 'matchMedia', { configurable: true, writable: true, value: matchMedia })
  return { list, matchMedia, change: (next: boolean) => { list.matches = next; listeners.forEach((listener) => listener()) } }
}

describe('useIsMobile', () => {
  it('shares one listener between all components and follows the window', () => {
    const media = mockMedia(true)
    const view = render(<><Probe /><Probe /><Probe /></>)
    expect(view.container.querySelectorAll('[data-phone="true"]')).toHaveLength(3)
    expect(media.list.addEventListener).toHaveBeenCalledTimes(1)
    act(() => media.change(false))
    expect(view.container.querySelectorAll('[data-phone="false"]')).toHaveLength(3)
    view.unmount()
    expect(media.list.removeEventListener).toHaveBeenCalledTimes(1)
  })

  it('is not a phone where matchMedia does not exist', () => {
    Reflect.deleteProperty(window, 'matchMedia')
    expect(render(<Probe />).container.querySelector('[data-phone="false"]')).toBeTruthy()
  })

  it('uses the same width as the stylesheet that presents dialogs', () => {
    const sheetCss = readFileSync(resolve(__dirname, 'sheet.css'), 'utf8')
    expect(sheetCss).toContain(`@media (max-width: ${String(MOBILE_MAX_WIDTH)}px)`)
  })
})
