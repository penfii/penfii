import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { MutationCache, QueryClient, onlineManager, useMutation } from '@tanstack/react-query'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createPenfiiClient, isApiError } from '@penfii/api-client'
import { OfflineWrites } from './offlineWrites'
import { queryClient } from './queryClient'
import { renderWithProviders } from '../test/renderWithProviders'

function setOnline(value: boolean) {
  Object.defineProperty(navigator, 'onLine', { configurable: true, value })
  act(() => { window.dispatchEvent(new Event(value ? 'online' : 'offline')) })
}

afterEach(() => { cleanup(); setOnline(true); onlineManager.setOnline(true); vi.unstubAllGlobals() })

describe('offline writes, on every shell', () => {
  it('refuses a write at the API client, says why once, and lets reads and online writes through', async () => {
    const fetchMock = vi.fn(() => Promise.resolve(new Response(JSON.stringify({ id: 'u' }), { status: 200 })))
    vi.stubGlobal('fetch', fetchMock)
    localStorage.setItem('penfii.locale', 'en-US')
    const client = createPenfiiClient()
    renderWithProviders(<OfflineWrites />)
    setOnline(false)
    const refused = await client.logout().catch((error: unknown) => error)
    expect(isApiError(refused) && refused.code === 'offline').toBe(true)
    expect(fetchMock).not.toHaveBeenCalled()
    expect((await screen.findByRole('status')).textContent).toMatch(/You are offline, so this change was not made/)
    await client.me()
    expect(fetchMock).toHaveBeenCalledTimes(1)
    setOnline(true)
    await waitFor(() => expect(screen.getByRole('status').textContent).toBe(''))
    await client.logout()
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('never queues a mutation: it fails at once and is not sent on reconnect', async () => {
    const write = vi.fn().mockRejectedValue(new Error('offline'))
    function Writer() {
      const m = useMutation({ mutationFn: write })
      return <button type="button" onClick={() => m.mutate()}>{m.isPaused ? 'paused' : m.isError ? 'failed' : 'idle'}</button>
    }
    const client = new QueryClient({ defaultOptions: queryClient.getDefaultOptions(), mutationCache: new MutationCache() })
    renderWithProviders(<Writer />, { client })
    act(() => onlineManager.setOnline(false))
    fireEvent.click(screen.getByRole('button'))
    await waitFor(() => expect(screen.getByRole('button').textContent).toBe('failed'))
    act(() => onlineManager.setOnline(true))
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(write).toHaveBeenCalledTimes(1)
  })
})
