export type SendPhase = 'sending' | 'failed' | 'sent'

export async function runTrackedSend<T>(options: {
  idempotencyKey: string
  execute: (idempotencyKey: string) => Promise<T>
  onPhase: (phase: SendPhase, idempotencyKey: string) => void
}): Promise<
  | { phase: 'sent'; idempotencyKey: string; value: T }
  | { phase: 'failed'; idempotencyKey: string }
> {
  options.onPhase('sending', options.idempotencyKey)
  try {
    const value = await options.execute(options.idempotencyKey)
    options.onPhase('sent', options.idempotencyKey)
    return { phase: 'sent', idempotencyKey: options.idempotencyKey, value }
  } catch {
    options.onPhase('failed', options.idempotencyKey)
    return { phase: 'failed', idempotencyKey: options.idempotencyKey }
  }
}
