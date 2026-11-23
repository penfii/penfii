import { afterEach, describe, expect, it, vi } from 'vitest'
import { createPenfiiClient, setUnauthorizedHandler } from './index'

afterEach(() => {
  vi.unstubAllGlobals()
  setUnauthorizedHandler(null)
})

function answer(code: string) {
  return vi.fn(() => Promise.resolve(new Response(JSON.stringify({ code, message: code }), { status: 401 })))
}

describe('a 401 from a two-step verification action', () => {
  it('is an answer when the code was wrong: the session stays', async () => {
    const ended = vi.fn()
    setUnauthorizedHandler(ended)
    vi.stubGlobal('fetch', answer('mfa_code_invalid'))
    await expect(createPenfiiClient().mfaDisable('123456')).rejects.toMatchObject({ status: 401, code: 'mfa_code_invalid' })
    expect(ended).not.toHaveBeenCalled()
  })

  it('still ends the session when the session itself is gone', async () => {
    const ended = vi.fn()
    setUnauthorizedHandler(ended)
    vi.stubGlobal('fetch', answer('unauthenticated'))
    await expect(createPenfiiClient().mfaEnable('123456')).rejects.toMatchObject({ status: 401, code: 'unauthenticated' })
    expect(ended).toHaveBeenCalledTimes(1)
  })
})
