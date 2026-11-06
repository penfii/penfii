import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useLongPress } from './useLongPress'
import { Sheet, SheetRow } from './Sheet'
import { I18nProvider } from '../i18n/I18nProvider'

function Target({ onPress, enabled = true }: { onPress: () => void; enabled?: boolean }) {
  return <div data-testid="t" {...useLongPress(onPress, enabled)} />
}

beforeEach(() => { vi.useFakeTimers(); localStorage.setItem('penfii.locale', 'en-US') })
afterEach(() => { cleanup(); vi.useRealTimers() })

describe('useLongPress', () => {
  it('fires after a touch is held still', () => {
    const onPress = vi.fn()
    render(<Target onPress={onPress} />)
    fireEvent.pointerDown(screen.getByTestId('t'), { pointerType: 'touch', clientX: 5, clientY: 5 })
    act(() => { vi.advanceTimersByTime(500) })
    expect(onPress).toHaveBeenCalledTimes(1)
  })

  it('does not fire when the finger moves, lifts early, or the pointer is a mouse', () => {
    const onPress = vi.fn()
    render(<Target onPress={onPress} />)
    const target = screen.getByTestId('t')
    fireEvent.pointerDown(target, { pointerType: 'touch', clientX: 5, clientY: 5 })
    fireEvent.pointerMove(target, { pointerType: 'touch', clientX: 40, clientY: 5 })
    act(() => { vi.advanceTimersByTime(600) })
    fireEvent.pointerDown(target, { pointerType: 'touch', clientX: 5, clientY: 5 })
    fireEvent.pointerUp(target, { pointerType: 'touch' })
    act(() => { vi.advanceTimersByTime(600) })
    fireEvent.pointerDown(target, { pointerType: 'mouse', clientX: 5, clientY: 5 })
    act(() => { vi.advanceTimersByTime(600) })
    expect(onPress).not.toHaveBeenCalled()
  })

  it('does not fire after the element is gone, or after the gesture is switched off, while a hold is running', () => {
    const onPress = vi.fn()
    const gone = render(<Target onPress={onPress} />)
    fireEvent.pointerDown(gone.getByTestId('t'), { pointerType: 'touch', clientX: 5, clientY: 5 })
    gone.unmount()
    act(() => { vi.advanceTimersByTime(600) })
    const off = render(<Target onPress={onPress} />)
    fireEvent.pointerDown(off.getByTestId('t'), { pointerType: 'touch', clientX: 5, clientY: 5 })
    off.rerender(<Target onPress={onPress} enabled={false} />)
    act(() => { vi.advanceTimersByTime(600) })
    expect(onPress).not.toHaveBeenCalled()
  })

  it('is inert when disabled', () => {
    const onPress = vi.fn()
    render(<Target onPress={onPress} enabled={false} />)
    fireEvent.pointerDown(screen.getByTestId('t'), { pointerType: 'touch', clientX: 5, clientY: 5 })
    act(() => { vi.advanceTimersByTime(600) })
    expect(onPress).not.toHaveBeenCalled()
  })
})

describe('Sheet', () => {
  it('renders its rows, marks the selected one and closes on Escape', () => {
    const onClose = vi.fn()
    const onPick = vi.fn()
    render(<I18nProvider><Sheet open title="Save" onClose={onClose}><SheetRow selected onClick={onPick}>Favorites</SheetRow></Sheet></I18nProvider>)
    expect(screen.getByRole('dialog', { name: 'Save' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /Favorites/ }))
    expect(onPick).toHaveBeenCalled()
    expect(screen.getByRole('button', { name: /Favorites/ }).getAttribute('aria-pressed')).toBe('true')
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalled()
  })
})
