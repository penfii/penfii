import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ConversationChange, Message, RichTextDoc } from '@penfii/contracts'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter } from 'react-router'
import { I18nProvider } from '../../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../../i18n/messages'
import { ConversationLiveProvider } from './live'
import { Timeline } from './Timeline'

const api = vi.hoisted(() => ({
  getConversation: vi.fn(),
  listMessages: vi.fn(),
  listChanges: vi.fn(),
  markConversationRead: vi.fn(),
  getSpace: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

class FakeSocket {
  static sockets: FakeSocket[] = []
  sent: string[] = []
  onopen: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null
  onclose: (() => void) | null = null
  constructor(readonly url: string) {
    FakeSocket.sockets.push(this)
    queueMicrotask(() => this.onopen?.())
  }
  send(data: string) {
    this.sent.push(data)
  }
  close() {
    this.onclose?.()
  }
  receive(frame: object) {
    this.onmessage?.({ data: JSON.stringify(frame) })
  }
}

class FakeObserver {
  static all: FakeObserver[] = []
  target: Element | null = null
  constructor(readonly callback: IntersectionObserverCallback) {
    FakeObserver.all.push(this)
  }
  observe(target: Element) {
    this.target = target
  }
  disconnect() {
    FakeObserver.all = FakeObserver.all.filter((observer) => observer !== this)
  }
  /** Scrolls the top sentinel into view. */
  static reachTop() {
    const observer = FakeObserver.all.find((item) => item.target?.classList.contains('older-sentinel'))
    const entry = { isIntersecting: true, intersectionRatio: 1 } as IntersectionObserverEntry
    observer?.callback([entry], observer as unknown as IntersectionObserver)
  }
}

function doc(text: string): RichTextDoc {
  return { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] } as RichTextDoc
}

function message(id: string, streamSeq: number, text: string, patch: Partial<Message> = {}): Message {
  return {
    id,
    streamSeq,
    authorId: 'u1',
    authorDisplayName: 'Ada',
    version: 1,
    deleted: false,
    body: doc(text),
    createdAt: '2026-11-05T00:00:00Z',
    editedAt: null,
    replyCount: 0,
    replyToId: null,
    threadRootId: null,
    reactions: [],
    attachments: [],
    ...patch,
  }
}

function conversation(id: string, lastChangeSeq: number) {
  return {
    id,
    generation: 1,
    readThroughSeq: 0,
    manualUnreadFromSeq: null,
    title: null,
    kind: 'CHANNEL',
    lastChangeSeq,
  }
}

function change(seq: number, messageId: string, changeKind: string): ConversationChange {
  return { conversationId: 'c1', changeSeq: seq, messageId, messageVersion: 2, changeKind }
}

function frame(changeItem: ConversationChange) {
  return { v: 1, type: 'conversation.change', ...changeItem }
}

function newest(count: number, start: number, text: (seq: number) => string): Message[] {
  return Array.from({ length: count }, (_, index) => {
    const streamSeq = start + index
    return message(`m${String(streamSeq)}`, streamSeq, text(streamSeq))
  })
}

function mount(conversationId = 'c1') {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[`/w/ws/c/${conversationId}`]}>
        <I18nProvider>
          <ConversationLiveProvider conversationId={conversationId}>
            <Timeline
              workspaceId="ws"
              conversationId={conversationId}
              currentUserId="u1"
              names={{}}
              showDmWarning={false}
              attempt={null}
              onRetry={() => undefined}
              onNotFound={() => undefined}
            />
          </ConversationLiveProvider>
        </I18nProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
  return view
}

function sentCursorCount(): number {
  return FakeSocket.sockets.reduce((count, socket) => count + socket.sent.length, 0)
}

async function sentHellos(): Promise<number[]> {
  let cursors: number[] = []
  await waitFor(() => {
    cursors = FakeSocket.sockets.flatMap((socket) => socket.sent.map((payload) => {
      const parsed = JSON.parse(payload) as { afterChangeSeq?: number }
      return parsed.afterChangeSeq
    })).filter((cursor): cursor is number => typeof cursor === 'number')
    expect(cursors.length).toBeGreaterThan(0)
  })
  return cursors
}

beforeEach(() => {
  FakeSocket.sockets = []
  vi.stubGlobal('WebSocket', FakeSocket)
  FakeObserver.all = []
  vi.stubGlobal('IntersectionObserver', FakeObserver)
  api.getConversation.mockReset()
  api.listMessages.mockReset()
  api.listChanges.mockReset()
  api.markConversationRead.mockReset()
  api.getSpace.mockReset()
  api.markConversationRead.mockResolvedValue({ generation: 1, readThroughSeq: 0, manualUnreadFromSeq: null })
  api.getSpace.mockResolvedValue({ teams: [], projects: [], channels: [], dms: [] })
  api.listChanges.mockResolvedValue([])
})

afterEach(() => {
  cleanup()
})

describe('conversation realtime cursor', () => {
  it('opens the socket at the snapshot cursor, after that snapshot and before history is required to include it', async () => {
    const order: string[] = []
    api.getConversation.mockImplementation(async () => {
      order.push('conversation')
      return conversation('c1', 40)
    })
    api.listMessages.mockImplementation(async () => {
      order.push('history')
      return [message('m1', 1, 'hello')]
    })
    mount()
    expect(await sentHellos()).toEqual([40])
    expect(await screen.findByText('hello')).toBeTruthy()
    expect(order[0]).toBe('conversation')
    expect(order.indexOf('history')).toBeGreaterThan(0)
  })

  it('resets the cursor when the conversation changes', async () => {
    api.getConversation.mockImplementation(async (id: string) => conversation(id, id === 'c1' ? 40 : 3))
    api.listMessages.mockResolvedValue([])
    const view = mount('c1')
    expect(await sentHellos()).toEqual([40])
    view.rerender(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <MemoryRouter initialEntries={['/w/ws/c/c2']}>
          <I18nProvider>
            <ConversationLiveProvider conversationId="c2">
              <Timeline
                workspaceId="ws"
                conversationId="c2"
                currentUserId="u1"
                names={{}}
                showDmWarning={false}
                attempt={null}
                onRetry={() => undefined}
                onNotFound={() => undefined}
              />
            </ConversationLiveProvider>
          </I18nProvider>
        </MemoryRouter>
      </QueryClientProvider>,
    )
    await waitFor(() => expect(sentCursorCount()).toBeGreaterThan(1))
    const cursors = await sentHellos()
    expect(cursors).toContain(3)
    expect(cursors.filter((cursor) => cursor === 40)).toHaveLength(1)
  })

  it('keeps hasOlder and applies changes that arrive while history is loading', async () => {
    let release: (page: Message[]) => void = () => undefined
    const history = new Promise<Message[]>((resolve) => {
      release = resolve
    })
    const calls: Array<{ beforeSeq?: number; limit: number }> = []
    api.getConversation.mockResolvedValue(conversation('c1', 10))
    api.listMessages.mockImplementation(async (_id: string, query: { beforeSeq?: number; limit: number }) => {
      calls.push(query)
      if (calls.length === 1) return history
      return [message('fresh', 101, 'arrived-later')]
    })
    mount()
    expect(await sentHellos()).toEqual([10])
    const socket = FakeSocket.sockets[0]
    socket?.receive(frame(change(11, 'fresh', 'message.created')))
    socket?.receive(frame(change(12, 'other', 'message.created')))
    release(newest(50, 1, (seq) => `base-${String(seq)}`))
    expect(await screen.findByText('arrived-later')).toBeTruthy()
    await waitFor(() => expect(document.querySelector('.older-sentinel')).toBeTruthy())
    expect(calls[0]?.limit).toBe(50)
    expect(calls[0]?.beforeSeq).toBeUndefined()
  })

  it('reconnects from the cursor the live frames advanced', async () => {
    api.getConversation.mockResolvedValue(conversation('c1', 40))
    api.listMessages.mockResolvedValue([message('m1', 1, 'hello')])
    mount()
    expect(await sentHellos()).toEqual([40])
    await screen.findByText('hello')
    const open = FakeSocket.sockets.find((socket) => socket.sent.length > 0)
    open?.receive(frame(change(80, 'm1', 'message.edited')))
    open?.close()
    await waitFor(() => expect(sentCursorCount()).toBeGreaterThan(1), { timeout: 3000 })
    expect(await sentHellos()).toEqual([40, 80])
  })

  it('updates every already-loaded older message in a burst', async () => {
    const window = newest(50, 51, () => 'recent')
    const older = [
      message('old-a', 1, 'alpha'),
      message('old-b', 2, 'beta'),
      message('old-c', 3, 'gamma'),
    ]
    api.getConversation.mockResolvedValue(conversation('c1', 100))
    api.listMessages.mockImplementation(async (_id: string, query: { beforeSeq?: number; limit: number }) => {
      if (query.beforeSeq == null) return window
      if (query.beforeSeq === 51) return older
      const seq = query.beforeSeq - 1
      const current = older.find((item) => item.streamSeq === seq)
      if (!current) return []
      if (current.id === 'old-b') return [{ ...current, deleted: true, body: null }]
      return [{ ...current, version: 2, body: doc(`${current.id}-edited`) }]
    })
    mount()
    expect(await sentHellos()).toEqual([100])
    await waitFor(() => expect(document.querySelector('.older-sentinel')).toBeTruthy())
    act(() => FakeObserver.reachTop())
    act(() => FakeObserver.reachTop())
    expect(await screen.findByText('alpha')).toBeTruthy()
    expect(api.listMessages.mock.calls.filter(([, query]) => query.beforeSeq === 51)).toHaveLength(1)
    expect(screen.getByText('beta')).toBeTruthy()
    const socket = FakeSocket.sockets[0]
    socket?.receive(frame(change(101, 'old-a', 'message.edited')))
    socket?.receive(frame(change(102, 'old-b', 'message.deleted')))
    socket?.receive(frame(change(103, 'old-c', 'message.edited')))
    expect(await screen.findByText('old-a-edited')).toBeTruthy()
    expect(await screen.findByText('old-c-edited')).toBeTruthy()
    const deleted = document.querySelector('[data-message-id="old-b"]')
    expect(deleted?.querySelector('[data-message-state="deleted"]')).toBeTruthy()
  })
})
