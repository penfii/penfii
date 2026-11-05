import { useEffect, useRef } from 'react'
import type { ReadRequest, ReadResult } from '@penfii/contracts'
import { isApiError, newIdempotencyKey } from '@penfii/api-client'
import { nextReadThroughSeq } from './readState'

export function useAutoRead(options: {
  active: boolean
  generation: number | null
  readThroughSeq: number
  visibleSeqs: readonly number[]
  ownSeqs: readonly number[]
  loadedSeqs: readonly number[]
  hasOlder: boolean
  manualUnread: boolean
  post: (body: ReadRequest, idempotencyKey: string) => Promise<ReadResult>
  refreshGeneration: () => Promise<number | null>
  onSuccess: (result: ReadResult) => void
  onStale: (generation: number) => void
}): { bumpEpoch: () => void } {
  const epoch = useRef(0)
  const staleKey = useRef<string | null>(null)
  const manualRef = useRef(options.manualUnread)
  const postRef = useRef(options.post)
  const refreshRef = useRef(options.refreshGeneration)
  const onSuccessRef = useRef(options.onSuccess)
  const onStaleRef = useRef(options.onStale)
  manualRef.current = options.manualUnread
  postRef.current = options.post
  refreshRef.current = options.refreshGeneration
  onSuccessRef.current = options.onSuccess
  onStaleRef.current = options.onStale

  const visibleKey = [...options.visibleSeqs].sort((left, right) => left - right).join(',')
  const ownKey = [...options.ownSeqs].sort((left, right) => left - right).join(',')
  const loadedKey = [...(options.loadedSeqs ?? [])].sort((left, right) => left - right).join(',')

  useEffect(() => {
    if (!options.active || options.generation == null || options.manualUnread) return
    const next = nextReadThroughSeq(options.readThroughSeq, options.visibleSeqs, options.ownSeqs, {
      // The effect follows the loaded Messages by value (`loadedKey`), not by array identity.
      loadedSeqs: loadedKey === '' ? [] : loadedKey.split(',').map(Number),
      hasOlder: options.hasOlder ?? false,
    })
    if (next == null) return
    const signature = `${String(options.generation)}:${String(next)}`
    if (staleKey.current === signature) return
    const ticket = epoch.current
    const generation = options.generation
    const handle = window.setTimeout(() => {
      void (async () => {
        try {
          const result = await postRef.current(
            {
              generation,
              readThroughSeq: next,
              scope: 'auto',
              observed: true,
            },
            newIdempotencyKey(),
          )
          if (ticket !== epoch.current || manualRef.current) return
          staleKey.current = null
          onSuccessRef.current(result)
        } catch (error) {
          if (!isApiError(error) || error.code !== 'stale_read') return
          const refreshed = await refreshRef.current()
          if (refreshed == null) return
          staleKey.current = `${String(refreshed)}:${String(next)}`
          onStaleRef.current(refreshed)
        }
      })()
    }, 400)
    return () => window.clearTimeout(handle)
  }, [
    options.active,
    options.generation,
    options.manualUnread,
    options.hasOlder,
    options.ownSeqs,
    options.readThroughSeq,
    options.visibleSeqs,
    loadedKey,
    ownKey,
    visibleKey,
  ])

  return {
    bumpEpoch: () => {
      epoch.current += 1
    },
  }
}
