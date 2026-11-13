// @vitest-environment jsdom
import { QueryClient } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPenfiiClient, getAuthEpoch, isApiError } from '@penfii/api-client'
import { dropReturn, peekReturn, rememberReturn, takeReturn } from './returnTo'
import { adoptUser, endUserSession, lastUserId, purgeUserStorage } from './userBoundary'

describe('the boundary between two people on one browser', () => {
  beforeEach(() => {
    localStorage.clear()
    sessionStorage.clear()
  })
  afterEach(() => vi.unstubAllGlobals())

  it('refuses the answer of a request that was asked under an earlier session', async () => {
    let finish: (response: Response) => void = () => undefined
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => { finish = resolve })))
    const queryClient = new QueryClient()
    const pending = createPenfiiClient().me().catch((caught: unknown) => caught)
    const before = getAuthEpoch()
    endUserSession(queryClient, { forget: false })
    expect(getAuthEpoch()).toBe(before + 1)
    finish(new Response(JSON.stringify({ id: 'u-a', email: 'a@example.test', displayName: 'A' }), { status: 200 }))
    const outcome = await pending
    expect(outcome).toBeDefined()
    expect(isApiError(outcome) ? outcome.code : (outcome as Error).name).toMatch(/session_changed|AbortError/)
  })

  it('aborts the request on the wire and clears the cache and the mutation cache', () => {
    let signal: AbortSignal | undefined
    vi.stubGlobal('fetch', vi.fn((_url: string, init: RequestInit) => { signal = init.signal ?? undefined; return new Promise<Response>(() => undefined) }))
    const queryClient = new QueryClient()
    queryClient.setQueryData(['space', 'w'], { channels: ['A private channel'] })
    void createPenfiiClient().me().catch(() => undefined)
    endUserSession(queryClient, { forget: false })
    expect(signal?.aborted).toBe(true)
    expect(queryClient.getQueryCache().getAll()).toHaveLength(0)
  })

  it('keeps the person\'s saved state when only the session expired, and drops it on sign out', () => {
    localStorage.setItem('penfii.doc.newDraft.v1:u-a:w', '{"title":"x"}')
    localStorage.setItem('penfii.theme', 'dark')
    endUserSession(new QueryClient(), { forget: false })
    expect(localStorage.getItem('penfii.doc.newDraft.v1:u-a:w')).not.toBeNull()
    endUserSession(new QueryClient(), { forget: true })
    expect(localStorage.getItem('penfii.doc.newDraft.v1:u-a:w')).toBeNull()
    expect(localStorage.getItem('penfii.theme')).toBe('dark')
  })

  it('drops the earlier person\'s saved state when someone else signs in, and keeps the device settings', () => {
    adoptUser('u-a')
    localStorage.setItem('penfii.ticketSeen.t1', 'x')
    sessionStorage.setItem('penfii.pendingDm', '[{"id":"u-x","displayName":"X"}]')
    localStorage.setItem('penfii.locale', 'en')
    expect(adoptUser('u-a')).toBe(false)
    expect(localStorage.getItem('penfii.ticketSeen.t1')).not.toBeNull()
    expect(adoptUser('u-b')).toBe(true)
    expect(localStorage.getItem('penfii.ticketSeen.t1')).toBeNull()
    expect(sessionStorage.getItem('penfii.pendingDm')).toBeNull()
    expect(localStorage.getItem('penfii.locale')).toBe('en')
    expect(lastUserId()).toBe('u-b')
  })

  it('purges every key that is not a device setting', () => {
    localStorage.setItem('penfii.work.view.p', 'v')
    localStorage.setItem('penfii.contextNavWidth', '300')
    localStorage.setItem('other.app', 'keep')
    purgeUserStorage()
    expect(localStorage.getItem('penfii.work.view.p')).toBeNull()
    expect(localStorage.getItem('penfii.contextNavWidth')).toBe('300')
    expect(localStorage.getItem('other.app')).toBe('keep')
  })
})

describe('the way back after signing in', () => {
  beforeEach(() => {
    localStorage.clear()
    sessionStorage.clear()
  })

  it('returns the same person to where they were and gives another person the fallback', () => {
    adoptUser('u-a')
    rememberReturn('/w/w1/c/private-room')
    expect(peekReturn()).toBe('/w/w1/c/private-room')
    expect(takeReturn('/', 'u-b')).toBe('/')
    expect(peekReturn()).toBeUndefined()
    rememberReturn('/w/w1/inbox')
    expect(takeReturn('/', 'u-a')).toBe('/w/w1/inbox')
  })

  it('keeps an invitation link for whoever opens it, and never an address outside the app', () => {
    rememberReturn('/invite/abc')
    expect(takeReturn('/', 'u-z')).toBe('/invite/abc')
    sessionStorage.setItem('penfii.afterLogin', '//evil.example/x')
    expect(peekReturn()).toBeUndefined()
    dropReturn()
  })
})
