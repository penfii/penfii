import { act, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'

const api = vi.hoisted(() => ({ checkProjectPrefix: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { useTicketPrefix } from './useTicketPrefix'

const wrapper = ({ children }: { children: ReactNode }) => <I18nProvider>{children}</I18nProvider>

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.checkProjectPrefix.mockReset()
})

describe('ticket prefix field', () => {
  it('follows the suggestion for the name until the person types their own', async () => {
    api.checkProjectPrefix.mockImplementation((_workspace: string, query: { name?: string; prefix?: string }) =>
      Promise.resolve({ suggestion: query.name === 'Penfii' ? 'PEN' : null, prefix: query.prefix ? { value: query.prefix.toUpperCase(), valid: true, available: query.prefix.toUpperCase() !== 'TAKEN' } : null }),
    )
    const { result, rerender } = renderHook(({ name }) => useTicketPrefix('w', name), { wrapper, initialProps: { name: 'Penfii' } })
    await waitFor(() => expect(result.current.value).toBe('PEN'))
    expect(result.current.usable).toBe(true)

    act(() => result.current.edit('taken'))
    expect(result.current.value).toBe('TAKEN')
    await waitFor(() => expect(result.current.hint).toContain('Another project already uses it'))
    expect(result.current.usable).toBe(false)

    // Once edited by hand, a changed name no longer overwrites the prefix.
    rerender({ name: 'Other name' })
    await new Promise((resolve) => setTimeout(resolve, 400))
    expect(result.current.value).toBe('TAKEN')

    act(() => result.current.edit('mine'))
    await waitFor(() => expect(result.current.usable).toBe(true))
  })

  it('asks for a typed prefix when the name gives no suggestion', async () => {
    api.checkProjectPrefix.mockResolvedValue({ suggestion: null, prefix: null })
    const { result } = renderHook(() => useTicketPrefix('w', '페니파이'), { wrapper })
    await waitFor(() => expect(result.current.hint).toContain('Type your own'))
    expect(result.current.usable).toBe(false)
  })
})
