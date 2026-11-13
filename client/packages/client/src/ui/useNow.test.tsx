import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useNow } from './useNow'

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-11-13T10:00:00Z'))
})
afterEach(() => vi.useRealTimers())

describe('useNow', () => {
  it('holds one value between ticks and moves it when the interval passes', () => {
    const { result } = renderHook(() => useNow(60_000))
    const first = result.current
    expect(first).toBe(Date.parse('2026-11-13T10:00:00Z'))
    act(() => { vi.advanceTimersByTime(59_000) })
    expect(result.current).toBe(first)
    act(() => { vi.advanceTimersByTime(1_000) })
    expect(result.current).toBe(first + 60_000)
  })

  it('reads the clock again when a hidden tab becomes visible', () => {
    const { result } = renderHook(() => useNow(60_000))
    const first = result.current
    vi.setSystemTime(new Date('2026-11-13T12:00:00Z'))
    act(() => { document.dispatchEvent(new Event('visibilitychange')) })
    expect(result.current).toBe(first + 2 * 3_600_000)
  })

  it('stops its timer when the component goes away', () => {
    const { unmount } = renderHook(() => useNow(1_000))
    unmount()
    expect(vi.getTimerCount()).toBe(0)
  })
})
