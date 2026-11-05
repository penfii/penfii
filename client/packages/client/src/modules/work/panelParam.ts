import { useCallback } from 'react'
import { useSearchParams } from 'react-router'
import type { HierarchyKind } from '@penfii/contracts'

export const TICKET_PANEL = 'ticket'
export const CREATE_PANEL = 'ticket-new'

export const CREATE_KEYS = ['newProject', 'newParent', 'newKind', 'newChannel', 'newAssignee', 'newFromConv', 'newFromMsg', 'newFromRoot'] as const

/** `?panel=ticket:<id>` is how a Ticket is opened beside the page it was opened from. */
export function ticketIdFromPanel(panel: string | null): string | null {
  if (!panel?.startsWith(`${TICKET_PANEL}:`)) return null
  return panel.slice(TICKET_PANEL.length + 1) || null
}

/** What the page that opened Create already knows. The Project is required; the rest is optional. */
export type CreateContext = {
  projectId: string
  parentId?: string
  kind?: HierarchyKind
  channelId?: string
  /** Who the new Ticket starts with (Create from Chat starts it with the person who asked). */
  assigneeId?: string
  /** The Chat message the Ticket is made from; its thread is told once the Ticket exists. */
  source?: ChatSource
}

export type ChatSource = { conversationId: string; messageId: string; threadRootId: string }

export function chatSourceFromParams(params: URLSearchParams): ChatSource | undefined {
  const conversationId = params.get('newFromConv')
  const messageId = params.get('newFromMsg')
  const threadRootId = params.get('newFromRoot')
  return conversationId && messageId && threadRootId ? { conversationId, messageId, threadRootId } : undefined
}

export function isCreatePanel(panel: string | null): boolean {
  return panel === CREATE_PANEL
}

export function createContextFromParams(params: URLSearchParams): CreateContext | null {
  const projectId = params.get('newProject')
  if (!projectId) return null
  const kind = params.get('newKind')
  return {
    projectId,
    parentId: params.get('newParent') ?? undefined,
    kind: kind === 'EPIC' || kind === 'TASK' || kind === 'SUBTASK' ? kind : undefined,
    channelId: params.get('newChannel') ?? undefined,
    assigneeId: params.get('newAssignee') ?? undefined,
    source: chatSourceFromParams(params),
  }
}

function withoutCreate(params: URLSearchParams): URLSearchParams {
  const next = new URLSearchParams(params)
  for (const key of CREATE_KEYS) next.delete(key)
  return next
}

export function useTicketPanel() {
  const [params, setParams] = useSearchParams()
  const selectedId = ticketIdFromPanel(params.get('panel'))
  const open = useCallback(
    (ticketId: string) => {
      const next = withoutCreate(params)
      next.set('panel', `${TICKET_PANEL}:${ticketId}`)
      setParams(next)
    },
    [params, setParams],
  )
  const close = useCallback(() => {
    const next = withoutCreate(params)
    next.delete('panel')
    setParams(next)
  }, [params, setParams])
  /** The one way every entry point (Board, a parent Ticket, a shortcut) starts a new Ticket. */
  const openCreate = useCallback(
    (context: CreateContext) => {
      const next = withoutCreate(params)
      next.set('panel', CREATE_PANEL)
      next.set('newProject', context.projectId)
      if (context.parentId) next.set('newParent', context.parentId)
      if (context.kind) next.set('newKind', context.kind)
      if (context.channelId) next.set('newChannel', context.channelId)
      if (context.assigneeId) next.set('newAssignee', context.assigneeId)
      if (context.source) {
        next.set('newFromConv', context.source.conversationId)
        next.set('newFromMsg', context.source.messageId)
        next.set('newFromRoot', context.source.threadRootId)
      }
      setParams(next)
    },
    [params, setParams],
  )
  return { selectedId, open, close, openCreate }
}
