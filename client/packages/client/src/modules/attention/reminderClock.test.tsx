import { act, cleanup, fireEvent, screen } from '@testing-library/react'
import type { LaterItem } from '@penfii/contracts'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ setReminder: vi.fn(), cancelReminder: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { ReminderControl } from './ReminderControl'
import { renderWithProviders } from '../../test/renderWithProviders'

const item = { id: 'i1', kind: 'MESSAGE', note: null, state: 'ACTIVE', version: 1, createdAt: '', updatedAt: '', reminder: null } as unknown as LaterItem

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  vi.useFakeTimers()
  vi.setSystemTime(new Date(2026, 10, 13, 10, 0, 0))
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

describe('the Reminder clock', () => {
  it('moves the earliest choosable time on as time passes, without another render being asked for', () => {
    renderWithProviders(<ReminderControl item={item} />)
    fireEvent.click(screen.getByRole('button', { name: /Remind me/ }))
    const input = () => screen.getByLabelText(/pick a date/i) as HTMLInputElement
    expect(input().min).toBe('2026-11-13T10:00')
    act(() => { vi.advanceTimersByTime(3 * 3_600_000) })
    expect(input().min).toBe('2026-11-13T13:00')
  })

  it('refuses a time that has meanwhile become the past', () => {
    renderWithProviders(<ReminderControl item={item} />)
    fireEvent.click(screen.getByRole('button', { name: /Remind me/ }))
    fireEvent.change(screen.getByLabelText(/pick a date/i), { target: { value: '2026-11-13T10:30' } })
    act(() => { vi.advanceTimersByTime(3 * 3_600_000) })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(api.setReminder).not.toHaveBeenCalled()
    expect(screen.getByText(/has passed/i)).toBeTruthy()
  })
})
