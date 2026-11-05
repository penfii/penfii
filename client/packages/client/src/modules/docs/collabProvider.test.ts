import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import * as awarenessProtocol from 'y-protocols/awareness'
import * as syncProtocol from 'y-protocols/sync'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'
import { CLOSE, DocCollabProvider, collabUrl, type SocketLike } from './collabProvider'

/** A tiny stand-in for the server: one authoritative Y.Doc, a lineage, and the same framing. */
class FakeServer {
  doc = new Y.Doc()
  epoch = 'epoch-1'
  sockets = new Set<FakeSocket>()
  /** Connections fail before the hello (the server is down). */
  refusing = false
  accept(socket: FakeSocket): void {
    this.sockets.add(socket)
    queueMicrotask(() => {
      if (this.refusing) {
        socket.drop(1006)
        return
      }
      socket.readyState = 1
      socket.deliverText(JSON.stringify({ type: 'hello', v: 1, epoch: this.epoch, role: 'EDITOR' }))
      const encoder = encoding.createEncoder()
      encoding.writeVarUint(encoder, 0)
      syncProtocol.writeSyncStep1(encoder, this.doc)
      socket.deliver(encoding.toUint8Array(encoder))
    })
  }
  receive(from: FakeSocket, data: Uint8Array): void {
    const decoder = decoding.createDecoder(data)
    const type = decoding.readVarUint(decoder)
    if (type !== 0) return
    const reply = encoding.createEncoder()
    encoding.writeVarUint(reply, 0)
    const before = Y.encodeStateVector(this.doc)
    syncProtocol.readSyncMessage(decoder, reply, this.doc, from)
    if (encoding.length(reply) > 1) from.deliver(encoding.toUint8Array(reply))
    const diff = Y.encodeStateAsUpdate(this.doc, before)
    for (const other of this.sockets) {
      if (other === from || other.readyState !== 1) continue
      const encoder = encoding.createEncoder()
      encoding.writeVarUint(encoder, 0)
      syncProtocol.writeUpdate(encoder, diff)
      other.deliver(encoding.toUint8Array(encoder))
    }
  }
}

class FakeSocket implements SocketLike {
  binaryType = 'blob'
  readyState = 0
  onopen: SocketLike['onopen'] = null
  onmessage: SocketLike['onmessage'] = null
  onclose: SocketLike['onclose'] = null
  onerror: SocketLike['onerror'] = null
  sent: Array<Uint8Array | string> = []
  constructor(readonly server: FakeServer, readonly url: string) {
    server.accept(this)
  }
  deliver(data: Uint8Array): void {
    this.onmessage?.({ data: data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) })
  }
  deliverText(text: string): void {
    this.onmessage?.({ data: text })
  }
  send(data: Uint8Array | string): void {
    this.sent.push(data)
    if (typeof data !== 'string') this.server.receive(this, data)
  }
  close(code = 1005): void {
    this.drop(code)
  }
  /** The server (or the network) ends the connection. */
  drop(code: number): void {
    this.readyState = 3
    this.server.sockets.delete(this)
    this.onclose?.({ code })
  }
}

function client(server: FakeServer, knownEpoch: string | null = null) {
  const doc = new Y.Doc()
  const awareness = new awarenessProtocol.Awareness(doc)
  const sockets: FakeSocket[] = []
  const provider = new DocCollabProvider({
    url: 'ws://test/v1/docs/p/collab',
    doc,
    awareness,
    knownEpoch,
    baseDelayMs: 1000,
    maxDelayMs: 30_000,
    random: () => 0,
    createSocket: (url) => {
      const socket = new FakeSocket(server, url)
      sockets.push(socket)
      return socket
    },
  })
  provider.connect()
  return { doc, awareness, provider, sockets }
}

const text = (doc: Y.Doc) => doc.getText('t').toString()
const flush = async () => {
  await Promise.resolve()
  await Promise.resolve()
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe('DocCollabProvider', () => {
  it('syncs both ways after the hello and stops saying "connecting"', async () => {
    const server = new FakeServer()
    server.doc.getText('t').insert(0, '서버 본문')
    const a = client(server)
    await flush()
    expect(a.provider.getState().status).toBe('live')
    expect(a.provider.getState().synced).toBe(true)
    expect(text(a.doc)).toBe('서버 본문')
    a.doc.getText('t').insert(0, '안녕 ')
    await flush()
    expect(text(server.doc)).toBe('안녕 서버 본문')
    a.provider.destroy()
  })

  it('shows another editor\'s typing and keeps both edits when they type at once', async () => {
    const server = new FakeServer()
    const a = client(server)
    const b = client(server)
    await flush()
    a.doc.getText('t').insert(0, 'A')
    b.doc.getText('t').insert(0, 'B')
    await flush()
    expect(text(a.doc)).toBe(text(b.doc))
    expect(text(a.doc)).toContain('A')
    expect(text(a.doc)).toContain('B')
    a.provider.destroy()
    b.provider.destroy()
  })

  it('lets the editor read pending input before a remote change is applied', async () => {
    const server = new FakeServer()
    const a = client(server)
    const b = client(server)
    await flush()
    const seen: string[] = []
    a.provider.beforeRemoteUpdate = () => {
      // At this moment the shared document still lacks the remote text.
      seen.push(text(a.doc))
    }
    b.doc.getText('t').insert(0, 'remote')
    await flush()
    expect(seen).toEqual([''])
    expect(text(a.doc)).toBe('remote')
    a.provider.destroy()
    b.provider.destroy()
  })

  it('reconnects with doubling delays after an unexpected close and merges offline edits once', async () => {
    const server = new FakeServer()
    const a = client(server)
    const b = client(server)
    await flush()
    server.refusing = true
    a.sockets[0]!.drop(1006)
    expect(a.provider.getState().status).toBe('reconnecting')
    a.doc.getText('t').insert(0, '오프라인 ')
    a.doc.getText('t').insert(0, '두번째 ')
    expect(a.provider.getState().unsynced).toBe(2)
    b.doc.getText('t').insert(0, '다른 사람 ')
    await flush()
    // The first retry comes after the base delay; a second drop doubles it.
    await vi.advanceTimersByTimeAsync(999)
    expect(a.sockets.length).toBe(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(a.sockets.length).toBe(2)
    // That attempt fails too: the next wait is doubled.
    await vi.advanceTimersByTimeAsync(1999)
    expect(a.sockets.length).toBe(2)
    server.refusing = false
    await vi.advanceTimersByTimeAsync(1)
    expect(a.sockets.length).toBe(3)
    await flush()
    expect(a.provider.getState().status).toBe('live')
    expect(a.provider.getState().unsynced).toBe(0)
    expect(text(a.doc)).toBe(text(b.doc))
    expect(text(a.doc).match(/오프라인/g)?.length).toBe(1)
    expect(text(a.doc).match(/다른 사람/g)?.length).toBe(1)
    a.provider.destroy()
    b.provider.destroy()
  })

  it('never reconnects after the server closes for access, deletion, size or session', async () => {
    for (const [code, reason] of [
      [CLOSE.REVOKED, 'revoked'],
      [CLOSE.GONE, 'deleted'],
      [CLOSE.TOO_LARGE, 'too-large'],
      [CLOSE.SESSION, 'session'],
    ] as const) {
      const server = new FakeServer()
      const a = client(server)
      await flush()
      a.sockets[0]!.drop(code)
      expect(a.provider.getState().status).toBe('closed')
      expect(a.provider.getState().closedReason).toBe(reason)
      await vi.advanceTimersByTimeAsync(120_000)
      expect(a.sockets.length).toBe(1)
      a.provider.destroy()
    }
  })

  it('does keep trying after other closes: restart, try later, too many connections', async () => {
    const server = new FakeServer()
    const a = client(server)
    await flush()
    for (const code of [CLOSE.RESTART, CLOSE.TRY_LATER, CLOSE.TOO_MANY]) {
      const before = a.sockets.length
      a.sockets[a.sockets.length - 1]!.drop(code)
      await vi.advanceTimersByTimeAsync(31_000)
      expect(a.sockets.length).toBeGreaterThan(before)
      await flush()
    }
    a.provider.destroy()
  })

  it('refuses to merge into a different lineage and asks to start over', async () => {
    const server = new FakeServer()
    const a = client(server)
    await flush()
    expect(a.provider.lineage).toBe('epoch-1')
    a.doc.getText('t').insert(0, '로컬 변경')
    a.sockets[0]!.drop(1006)
    server.epoch = 'epoch-2'
    server.doc = new Y.Doc()
    await vi.advanceTimersByTimeAsync(1000)
    await flush()
    expect(a.provider.getState().status).toBe('epoch-reset')
    expect(text(server.doc), 'the old document was not merged into the new lineage').toBe('')
    a.provider.destroy()
  })

  it('tracks saving from the materialized frame and shows title and comments signals', async () => {
    const server = new FakeServer()
    const a = client(server)
    await flush()
    a.doc.getText('t').insert(0, 'x')
    expect(a.provider.getState().saving).toBe(true)
    const socket = a.sockets[0]!
    socket.deliverText(JSON.stringify({ type: 'materialized', revisionNo: 4 }))
    expect(a.provider.getState().saving).toBe(false)
    expect(a.provider.getState().savedRevision).toBe(4)
    socket.deliverText(JSON.stringify({ type: 'meta', title: '새 제목' }))
    expect(a.provider.getState().title).toBe('새 제목')
    socket.deliverText(JSON.stringify({ type: 'comments_changed' }))
    expect(a.provider.getState().commentsTick).toBe(1)
    a.provider.destroy()
  })

  it('sends the cursor and renews it so other tabs keep showing it', async () => {
    const server = new FakeServer()
    const a = client(server)
    a.awareness.setLocalState({ cursor: { anchor: 1 } })
    await flush()
    const awarenessFrames = () => a.sockets[0]!.sent.filter((d) => typeof d !== 'string' && d[0] === 1).length
    const sentAfterJoin = awarenessFrames()
    expect(sentAfterJoin).toBeGreaterThan(0)
    await vi.advanceTimersByTimeAsync(16_000)
    expect(awarenessFrames()).toBeGreaterThan(sentAfterJoin)
    a.provider.destroy()
  })

  it('builds the socket address from the page location', () => {
    expect(collabUrl('abc', { protocol: 'http:', host: '127.0.0.1:5173' })).toBe('ws://127.0.0.1:5173/v1/docs/abc/collab')
    expect(collabUrl('abc', { protocol: 'https:', host: 'penfii.example.com' })).toBe('wss://penfii.example.com/v1/docs/abc/collab')
  })
})
