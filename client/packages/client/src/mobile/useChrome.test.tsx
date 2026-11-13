import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { useChromeCollapse, useKeyboardViewport } from './useChrome'

afterEach(cleanup)

function scrollTo(el: HTMLElement, top: number) {
  el.scrollTop = top
  fireEvent.scroll(el)
}

function Harness({ locationKey = 'a' }: { locationKey?: string }) {
  const { collapsed, onScroll } = useChromeCollapse(locationKey)
  return (
    <div data-collapsed={collapsed ? 'yes' : 'no'} onScrollCapture={onScroll}>
      <div className="board-viewport" data-testid="board" />
    </div>
  )
}

function tall(el: HTMLElement) {
  Object.defineProperty(el, 'scrollHeight', { configurable: true, value: 2000 })
  Object.defineProperty(el, 'clientHeight', { configurable: true, value: 300 })
}

describe('chrome collapse on scroll', () => {
  it('hides on scrolling down, shows on scrolling up and near the top', () => {
    const { container, getByTestId } = render(<Harness />)
    const board = getByTestId('board')
    const state = () => container.firstElementChild?.getAttribute('data-collapsed')
    tall(board)
    act(() => scrollTo(board, 5))
    act(() => scrollTo(board, 80))
    expect(state()).toBe('yes')
    act(() => scrollTo(board, 60))
    expect(state()).toBe('no')
    act(() => scrollTo(board, 200))
    expect(state()).toBe('yes')
    act(() => scrollTo(board, 10))
    expect(state()).toBe('no')
  })

  it('ignores a tremor and a scroller with no room to spare', () => {
    const { container, getByTestId } = render(<Harness />)
    const board = getByTestId('board')
    const state = () => container.firstElementChild?.getAttribute('data-collapsed')
    tall(board)
    act(() => scrollTo(board, 100))
    act(() => scrollTo(board, 105))
    expect(state()).toBe('no')
    Object.defineProperty(board, 'scrollHeight', { configurable: true, value: 400 })
    act(() => scrollTo(board, 160))
    expect(state()).toBe('no')
  })

  it('starts expanded again at a new location', () => {
    const { container, getByTestId, rerender } = render(<Harness />)
    const board = getByTestId('board')
    tall(board)
    act(() => scrollTo(board, 5))
    act(() => scrollTo(board, 90))
    expect(container.firstElementChild?.getAttribute('data-collapsed')).toBe('yes')
    rerender(<Harness locationKey="b" />)
    expect(container.firstElementChild?.getAttribute('data-collapsed')).toBe('no')
  })
})

describe('keyboard viewport', () => {
  it('follows the visual viewport and says when the keyboard is open', () => {
    const listeners: Record<string, () => void> = {}
    const viewport = { height: 800, offsetTop: 0, addEventListener: (name: string, fn: () => void) => { listeners[name] = fn }, removeEventListener: () => undefined }
    Object.defineProperty(window, 'visualViewport', { configurable: true, value: viewport })
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 800 })
    const { result, unmount } = renderHook(() => useKeyboardViewport())
    expect(result.current).toBe(false)
    expect(document.documentElement.style.getPropertyValue('--m-vh')).toBe('800px')
    viewport.height = 480
    act(() => listeners.resize())
    expect(result.current).toBe(true)
    expect(document.documentElement.style.getPropertyValue('--m-vh')).toBe('480px')
    unmount()
    expect(document.documentElement.style.getPropertyValue('--m-vh')).toBe('')
    Reflect.deleteProperty(window, 'visualViewport')
  })
})
