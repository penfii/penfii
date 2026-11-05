import { describe, expect, it } from 'vitest'
import { runTrackedSend } from './sendState'

describe('runTrackedSend', () => {
  it('stays in sending until the promise settles and keeps the idempotency key after rejection', async () => {
    const phases: Array<'sending' | 'failed' | 'sent'> = []
    let rejectFirst: (error: unknown) => void = () => undefined
    const first = new Promise<string>((_resolve, reject) => {
      rejectFirst = reject
    })
    const key = 'idem-fixed'
    const pending = runTrackedSend({
      idempotencyKey: key,
      execute: () => first,
      onPhase: (phase) => {
        phases.push(phase)
      },
    })

    expect(phases).toEqual(['sending'])
    await Promise.resolve()
    await Promise.resolve()
    expect(phases).toEqual(['sending'])

    rejectFirst(new Error('network'))
    const failed = await pending
    expect(failed.phase).toBe('failed')
    expect(failed.idempotencyKey).toBe(key)
    expect(phases).toEqual(['sending', 'failed'])

    const retryPhases: Array<'sending' | 'failed' | 'sent'> = []
    let resolveRetry: (value: string) => void = () => undefined
    const second = new Promise<string>((resolve) => {
      resolveRetry = resolve
    })
    const pendingRetry = runTrackedSend({
      idempotencyKey: failed.idempotencyKey,
      execute: (idempotencyKey) => {
        expect(idempotencyKey).toBe(key)
        return second
      },
      onPhase: (phase) => {
        retryPhases.push(phase)
      },
    })

    expect(retryPhases).toEqual(['sending'])
    await Promise.resolve()
    expect(retryPhases).toEqual(['sending'])
    resolveRetry('ok')
    const sent = await pendingRetry
    expect(sent.phase).toBe('sent')
    expect(sent.idempotencyKey).toBe(key)
    expect(retryPhases).toEqual(['sending', 'sent'])
  })
})
