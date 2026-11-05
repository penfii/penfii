import { useRef, useState } from 'react'
import type { RichTextDoc } from '@penfii/contracts'
import { flushSync } from 'react-dom'
import { newIdempotencyKey } from '@penfii/api-client'
import { runTrackedSend } from './sendState'

export type OutboundAttempt = {
  phase: 'sending' | 'failed'
  doc: RichTextDoc
  idempotencyKey: string
}

export function useOutboundMessage() {
  const [attempt, setAttempt] = useState<OutboundAttempt | null>(null)
  // The in-flight send key stays in memory until the post succeeds.
  const keyRef = useRef<string | null>(null)
  const sendingRef = useRef(false)

  async function submit(doc: RichTextDoc, execute: (idempotencyKey: string) => Promise<void>): Promise<boolean> {
    if (sendingRef.current) return false
    const idempotencyKey = keyRef.current ?? newIdempotencyKey()
    keyRef.current = idempotencyKey
    sendingRef.current = true
    flushSync(() => {
      setAttempt({ phase: 'sending', doc, idempotencyKey })
    })
    const result = await runTrackedSend({
      idempotencyKey,
      execute,
      onPhase: (phase) => {
        if (phase === 'failed') setAttempt({ phase: 'failed', doc, idempotencyKey })
      },
    })
    sendingRef.current = false
    if (result.phase === 'sent') {
      keyRef.current = null
      setAttempt(null)
      return true
    }
    keyRef.current = result.idempotencyKey
    setAttempt({ phase: 'failed', doc, idempotencyKey: result.idempotencyKey })
    return false
  }

  return { attempt, submit }
}
