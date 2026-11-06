import { afterEach, describe, expect, it, vi } from 'vitest'
import { setWriteBlocker } from '@penfii/api-client'
import { putFile } from './upload'

afterEach(() => { setWriteBlocker(null); vi.unstubAllGlobals() })

describe('putFile while writes are blocked', () => {
  it('fails at once with the offline error and opens no request', async () => {
    const open = vi.fn()
    vi.stubGlobal('XMLHttpRequest', class { upload = {}; open = open; setRequestHeader = vi.fn(); send = vi.fn(); abort = vi.fn() })
    setWriteBlocker(() => true)
    await expect(putFile('f', new File(['x'], 'x.txt'), () => undefined).done).rejects.toMatchObject({ code: 'offline' })
    expect(open).not.toHaveBeenCalled()
  })
})
