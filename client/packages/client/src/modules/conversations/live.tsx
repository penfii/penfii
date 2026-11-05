import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import type { Conversation, ConversationChange, Message, ServerFrame } from '@penfii/contracts'
import { api } from '../../api'
import { readAllChanges } from './realtime'

type LiveContextValue = {
  /** How many changes have been queued for this conversation. A burst grows this by the whole batch. */
  changeCount: number
  resyncCount: number
  /** Every change since this conversation was opened, in arrival order. Reset when the conversation changes. */
  changes: readonly ConversationChange[]
  /** Conversation snapshot taken before the socket opens. Null until that read finishes. */
  snapshot: Conversation | null
  snapshotError: unknown
  catalog: ReadonlyMap<string, Message>
  publish: (messages: readonly Message[]) => void
}

const LiveContext = createContext<LiveContextValue | null>(null)

function parseFrame(data: string): ServerFrame | null {
  try {
    const value = JSON.parse(data) as unknown
    if (typeof value !== 'object' || value === null) return null
    const frame = value as Partial<ServerFrame>
    if (frame.v !== 1) return null
    if (frame.type === 'resync_required') return { v: 1, type: 'resync_required' }
    if (
      frame.type === 'conversation.change' &&
      typeof frame.changeSeq === 'number' &&
      typeof frame.messageId === 'string'
    ) {
      return {
        v: 1,
        type: 'conversation.change',
        conversationId: typeof frame.conversationId === 'string' ? frame.conversationId : '',
        changeSeq: frame.changeSeq,
        messageId: frame.messageId,
        messageVersion: typeof frame.messageVersion === 'number' ? frame.messageVersion : 0,
        changeKind: typeof frame.changeKind === 'string' ? frame.changeKind : '',
      }
    }
    return null
  } catch {
    return null
  }
}

function asChange(frame: Extract<ServerFrame, { type: 'conversation.change' }>): ConversationChange {
  return {
    conversationId: frame.conversationId,
    changeSeq: frame.changeSeq,
    messageId: frame.messageId,
    messageVersion: frame.messageVersion,
    changeKind: frame.changeKind,
  }
}

export function ConversationLiveProvider({
  conversationId,
  children,
}: {
  conversationId: string
  children: ReactNode
}) {
  const afterRef = useRef(0)
  const generation = useRef(0)
  const bufferRef = useRef<ConversationChange[]>([])
  const [trackedId, setTrackedId] = useState(conversationId)
  const [snapshot, setSnapshot] = useState<Conversation | null>(null)
  const [snapshotError, setSnapshotError] = useState<unknown>(null)
  const [changes, setChanges] = useState<ConversationChange[]>([])
  const [resyncCount, setResyncCount] = useState(0)
  const [catalog, setCatalog] = useState<ReadonlyMap<string, Message>>(new Map())

  // Drop the previous conversation before children render, so its cursor cannot be sent for the next one.
  if (trackedId !== conversationId) {
    setTrackedId(conversationId)
    setSnapshot(null)
    setSnapshotError(null)
    setChanges([])
    setResyncCount(0)
    setCatalog(new Map())
    afterRef.current = 0
    bufferRef.current = []
    generation.current += 1
  }

  const publish = useCallback((messages: readonly Message[]) => {
    setCatalog((current) => {
      let changed = false
      const next = new Map(current)
      for (const message of messages) {
        if (current.get(message.id) === message) continue
        next.set(message.id, message)
        changed = true
      }
      return changed ? next : current
    })
  }, [])

  useEffect(() => {
    const epoch = generation.current
    let socket: WebSocket | null = null
    let closed = false
    let timer = 0
    let flushTimer = 0
    let startTimer = 0
    const flush = () => {
      flushTimer = 0
      if (generation.current !== epoch) {
        bufferRef.current = []
        return
      }
      const batch = bufferRef.current
      bufferRef.current = []
      if (batch.length === 0) return
      setChanges((current) => current.concat(batch))
    }
    const enqueue = (batch: readonly ConversationChange[]) => {
      if (generation.current !== epoch || batch.length === 0) return
      for (const change of batch) afterRef.current = Math.max(afterRef.current, change.changeSeq)
      bufferRef.current.push(...batch)
      if (flushTimer) return
      flushTimer = window.setTimeout(flush, 0)
    }
    const connect = () => {
      if (closed || generation.current !== epoch || typeof WebSocket !== 'function') return
      const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
      socket = new WebSocket(
        `${protocol}//${window.location.host}/v1/ws?conversationId=${encodeURIComponent(conversationId)}`,
      )
      socket.onopen = () => {
        if (generation.current !== epoch) return
        socket?.send(JSON.stringify({ v: 1, type: 'hello', afterChangeSeq: afterRef.current }))
      }
      socket.onmessage = (event) => {
        if (generation.current !== epoch || typeof event.data !== 'string') return
        const frame = parseFrame(event.data)
        if (!frame) return
        if (frame.type === 'resync_required') {
          setResyncCount((count) => count + 1)
          const from = afterRef.current
          void readAllChanges(conversationId, from)
            .then((caughtUp) => {
              if (generation.current !== epoch) return
              enqueue(caughtUp)
            })
            .catch(() => undefined)
          return
        }
        enqueue([asChange(frame)])
      }
      socket.onclose = () => {
        if (closed || generation.current !== epoch) return
        timer = window.setTimeout(connect, 1500)
      }
    }
    void api
      .getConversation(conversationId)
      .then((conversation) => {
        if (closed || generation.current !== epoch) return
        afterRef.current = conversation.lastChangeSeq
        setSnapshot(conversation)
        setSnapshotError(null)
        // The cursor is the snapshot's. Connecting before this read would replay the whole log.
        startTimer = window.setTimeout(connect, 0)
      })
      .catch((caught: unknown) => {
        if (closed || generation.current !== epoch) return
        setSnapshotError(caught)
      })
    return () => {
      closed = true
      window.clearTimeout(startTimer)
      window.clearTimeout(timer)
      window.clearTimeout(flushTimer)
      bufferRef.current = []
      socket?.close()
    }
  }, [conversationId])

  const value = useMemo<LiveContextValue>(() => {
    return {
      changeCount: changes.length,
      resyncCount,
      changes,
      snapshot,
      snapshotError,
      catalog,
      publish,
    }
  }, [catalog, changes, publish, resyncCount, snapshot, snapshotError])

  return <LiveContext.Provider value={value}>{children}</LiveContext.Provider>
}

export function useConversationLive(): LiveContextValue {
  const value = useContext(LiveContext)
  if (!value) throw new Error('ConversationLiveProvider is required')
  return value
}

export function useRegisteredMessage(id: string | null): Message | null {
  const { catalog } = useConversationLive()
  if (!id) return null
  return catalog.get(id) ?? null
}
