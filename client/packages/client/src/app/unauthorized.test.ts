import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPenfiiClient, isApiError, setUnauthorizedHandler } from '@penfii/api-client'

const unauthenticated = () =>
  new Response(JSON.stringify({ code: 'unauthenticated', message: 'unauthenticated' }), {
    status: 401,
    headers: { 'Content-Type': 'application/json' },
  })

describe('a 401 and the sign-in redirect', () => {
  const handler = vi.fn()

  beforeEach(() => {
    handler.mockReset()
    setUnauthorizedHandler(handler)
    vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(unauthenticated())))
  })

  afterEach(() => {
    setUnauthorizedHandler(null)
    vi.unstubAllGlobals()
  })

  it('does not end the session when it answers a wrong current password', async () => {
    const api = createPenfiiClient()
    const failure = await api
      .updateMe({ displayName: 'Dana', email: 'dana@example.com', currentPassword: 'wrong-password-123456' })
      .catch((caught: unknown) => caught)
    expect(isApiError(failure) && failure.status).toBe(401)
    expect(handler).not.toHaveBeenCalled()
  })

  it('still sends the person to sign in when a request without a password is refused', async () => {
    const api = createPenfiiClient()
    await api.updateMe({ displayName: 'Dana', email: 'dana@example.com' }).catch(() => undefined)
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('still sends the person to sign in when the session probe itself is refused', async () => {
    await createPenfiiClient().me().catch(() => undefined)
    expect(handler).toHaveBeenCalledTimes(1)
  })
})
