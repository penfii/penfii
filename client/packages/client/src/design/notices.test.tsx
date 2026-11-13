import { act, cleanup, fireEvent, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { NoticeHost, SUCCESS_MS, UNDO_MS, announceToggle, dismissNotice, notify, noticeDuration } from './notices'
import { renderWithProviders } from '../test/renderWithProviders'

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  vi.useFakeTimers()
})
afterEach(() => { cleanup(); vi.useRealTimers() })

const show = (input: Parameters<typeof notify>[0]) => act(() => { notify(input) })
const wait = (ms: number) => act(() => { vi.advanceTimersByTime(ms) })

describe('result notices', () => {
  it('is a polite status region that is there before anything is announced', () => {
    renderWithProviders(<NoticeHost />)
    const region = screen.getByRole('status')
    expect(region.getAttribute('aria-live')).toBe('polite')
    show({ message: 'Saved' })
    expect(screen.getByRole('status').textContent).toBe('Saved')
  })

  it('closes a plain result after about four seconds and one with Undo after about eight', () => {
    expect(SUCCESS_MS).toBe(4000)
    expect(UNDO_MS).toBe(8000)
    renderWithProviders(<NoticeHost />)
    show({ message: 'Done' })
    wait(SUCCESS_MS - 1)
    expect(screen.getByRole('status').textContent).toBe('Done')
    wait(1)
    expect(screen.getByRole('status').textContent).toBe('')
    show({ message: 'Removed', action: { label: 'Undo', run: () => undefined } })
    wait(SUCCESS_MS + 1)
    expect(screen.getByRole('status').textContent).toContain('Removed')
    wait(UNDO_MS - SUCCESS_MS)
    expect(screen.getByRole('status').textContent).toBe('')
  })

  it('replaces the notice before it', () => {
    renderWithProviders(<NoticeHost />)
    show({ message: 'First' })
    wait(3000)
    show({ message: 'Second' })
    expect(screen.queryByText('First')).toBeNull()
    // The second one has its own full time, not what was left of the first.
    wait(3000)
    expect(screen.getByText('Second')).toBeTruthy()
    wait(1000)
    expect(screen.queryByText('Second')).toBeNull()
  })

  it('waits while the pointer or keyboard focus is on it, then starts again', () => {
    renderWithProviders(<NoticeHost />)
    show({ message: 'Removed', action: { label: 'Undo', run: () => undefined } })
    const region = screen.getByRole('status')
    fireEvent.mouseEnter(region)
    wait(UNDO_MS * 3)
    expect(screen.getByText('Removed')).toBeTruthy()
    fireEvent.mouseLeave(region)
    wait(UNDO_MS - 1)
    expect(screen.getByText('Removed')).toBeTruthy()
    wait(1)
    expect(screen.queryByText('Removed')).toBeNull()

    show({ message: 'Again', action: { label: 'Undo', run: () => undefined } })
    const undo = screen.getByRole('button', { name: 'Undo' })
    fireEvent.focus(undo)
    wait(UNDO_MS * 2)
    expect(screen.getByText('Again')).toBeTruthy()
    fireEvent.blur(undo)
    wait(UNDO_MS)
    expect(screen.queryByText('Again')).toBeNull()
  })

  it('keeps an error until it is dismissed', () => {
    expect(noticeDuration({ kind: 'error', message: 'x' })).toBeNull()
    renderWithProviders(<NoticeHost />)
    show({ kind: 'error', message: 'Could not save' })
    wait(60_000)
    expect(screen.getByText('Could not save')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByText('Could not save')).toBeNull()
  })

  it('keeps a sticky result until it is dismissed, with or without an action', () => {
    expect(noticeDuration({ message: 'x', sticky: true })).toBeNull()
    renderWithProviders(<NoticeHost />)
    show({ message: 'Mina is now Admin', sticky: true })
    wait(60_000)
    expect(screen.getByText('Mina is now Admin')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    expect(screen.queryByText('Mina is now Admin')).toBeNull()
  })

  it('runs the action once from the keyboard-reachable button and closes first', () => {
    const run = vi.fn()
    renderWithProviders(<NoticeHost />)
    show({ message: 'Removed', action: { label: 'Undo', run } })
    const undo = screen.getByRole('button', { name: 'Undo' })
    expect(undo.tagName).toBe('BUTTON')
    fireEvent.click(undo)
    expect(run).toHaveBeenCalledTimes(1)
    expect(screen.queryByText('Removed')).toBeNull()
  })

  it('closes on Escape from inside the notice', () => {
    renderWithProviders(<NoticeHost />)
    show({ message: 'Removed', action: { label: 'Undo', run: () => undefined } })
    fireEvent.keyDown(screen.getByRole('button', { name: 'Undo' }), { key: 'Escape' })
    expect(screen.queryByText('Removed')).toBeNull()
    act(() => dismissNotice())
  })
})

describe('announceToggle', () => {
  const messages = { on: 'On', off: 'Off', error: 'Failed', undo: 'Undo' }

  it('says a plain result when turned on', async () => {
    renderWithProviders(<NoticeHost />)
    const apply = vi.fn().mockResolvedValue(undefined)
    await act(async () => { await announceToggle(true, apply, messages) })
    expect(apply).toHaveBeenCalledWith(true)
    expect(screen.getByRole('status').textContent).toBe('On')
  })

  it('offers Undo when turned off, and Undo turns it back on without offering Undo again', async () => {
    renderWithProviders(<NoticeHost />)
    const apply = vi.fn().mockResolvedValue(undefined)
    await act(async () => { await announceToggle(false, apply, messages) })
    expect(screen.getByRole('status').textContent).toContain('Off')
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Undo' })) })
    expect(apply).toHaveBeenLastCalledWith(true)
    expect(screen.getByRole('status').textContent).toBe('On')
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull()
  })

  it('keeps a failure on screen', async () => {
    renderWithProviders(<NoticeHost />)
    await act(async () => { await announceToggle(true, vi.fn().mockRejectedValue(new Error('no')), messages) })
    wait(60_000)
    expect(screen.getByText('Failed')).toBeTruthy()
  })
})
