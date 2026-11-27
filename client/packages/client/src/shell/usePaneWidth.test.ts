// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react'
import type { PointerEvent as ReactPointerEvent } from 'react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { THREAD_WIDTH_KEY } from './paneWidth'
import { usePaneWidth } from './usePaneWidth'

function setViewport(width: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width })
  act(() => {
    window.dispatchEvent(new Event('resize'))
  })
}

const press = (clientX: number, timeStamp: number) =>
  ({ clientX, timeStamp, pointerId: 1, preventDefault() {} }) as unknown as ReactPointerEvent<HTMLElement>
function pointer(type: string, clientX: number) {
  const event = new Event(type) as Event & { clientX: number; pointerId: number }
  Object.assign(event, { clientX, pointerId: 1 })
  act(() => {
    window.dispatchEvent(event)
  })
}

const thread = () => renderHook(() => usePaneWidth('thread', { threadOpen: true, panel: null }))

beforeEach(() => {
  localStorage.clear()
  setViewport(1024)
})
afterEach(() => {
  cleanup()
  localStorage.clear()
})

describe('thread width', () => {
  it('opens fitted to a narrow desktop while the person has not set a width, and at 420 once the window allows it', () => {
    const { result } = thread()
    // 1024 - rail 76 - "Show sidebar" strip 24 - main 640.
    expect(result.current.width).toBe(284)
    setViewport(1280)
    expect(result.current.width).toBe(420)
    expect(localStorage.getItem(THREAD_WIDTH_KEY)).toBeNull()
  })

  it('keeps a stored width as it is (the main column scrolls sideways instead)', () => {
    localStorage.setItem(THREAD_WIDTH_KEY, '440')
    const { result } = thread()
    expect(result.current.width).toBe(440)
    expect(localStorage.getItem(THREAD_WIDTH_KEY)).toBe('440')
  })

  it('takes a stored 420 for the default it is (earlier versions stored it on a reset) and leaves it stored', () => {
    localStorage.setItem(THREAD_WIDTH_KEY, '420')
    const { result } = thread()
    expect(result.current.width).toBe(284)
    expect(localStorage.getItem(THREAD_WIDTH_KEY)).toBe('420')
  })

  it('stores nothing for a press without a move, and drags from the width on screen', () => {
    const { result } = thread()
    act(() => result.current.beginDrag(press(500, 1_000), -1))
    pointer('pointerup', 500)
    expect(localStorage.getItem(THREAD_WIDTH_KEY)).toBeNull()
    act(() => result.current.beginDrag(press(500, 5_000), -1))
    pointer('pointermove', 490)
    pointer('pointerup', 490)
    expect(result.current.width).toBe(294)
    expect(localStorage.getItem(THREAD_WIDTH_KEY)).toBe('294')
  })

  it('goes back to the fitted default and forgets the stored width after a second press', () => {
    localStorage.setItem(THREAD_WIDTH_KEY, '600')
    const { result } = thread()
    expect(result.current.width).toBe(600)
    act(() => result.current.beginDrag(press(500, 1_000), -1))
    pointer('pointerup', 500)
    act(() => result.current.beginDrag(press(500, 1_200), -1))
    expect(result.current.width).toBe(284)
    expect(localStorage.getItem(THREAD_WIDTH_KEY)).toBeNull()
  })

  it('moves a key step from the width on screen and remembers it', () => {
    const { result } = thread()
    act(() => result.current.nudge(16))
    expect(result.current.width).toBe(300)
    expect(localStorage.getItem(THREAD_WIDTH_KEY)).toBe('300')
  })

  it('follows the Ticket panel beside it while that is resized, so the thread\'s handle stays in view', () => {
    setViewport(1440)
    const ticket = renderHook(() => usePaneWidth('ticket', { threadOpen: false, panel: null }))
    const { result } = renderHook(() => usePaneWidth('thread', { threadOpen: true, panel: 'ticket:a' }))
    expect(result.current.width).toBe(280)
    act(() => ticket.result.current.nudge(400))
    expect(ticket.result.current.width).toBe(880)
    // 1440 - rail 76 - strip 24 - Ticket panel 880 - 320 of the work area kept in view.
    expect(result.current.width).toBe(140)
  })
})
