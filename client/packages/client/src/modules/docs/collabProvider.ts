import * as Y from 'yjs'
import * as awarenessProtocol from 'y-protocols/awareness'
import * as syncProtocol from 'y-protocols/sync'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'

/**
 * The client half of Doc co-editing. It speaks y-protocols sync and awareness
 * over one WebSocket and adds what the server's control frames need: the lineage (`epoch`) rule,
 * close-code handling, reconnect with backoff, and the "saved" signal from materialization.
 *
 * It owns no document: the caller gives it a `Y.Doc` and an `Awareness`. While the socket is down
 * the document stays editable in memory; on reconnect sync step 1/2 sends only what the server lacks,
 * so edits made offline merge instead of overwriting. Nothing is persisted locally.
 */

const MESSAGE_SYNC = 0
const MESSAGE_AWARENESS = 1

/** Close codes the server uses (section F.3). */
export const CLOSE = {
  GOING_AWAY: 1001,
  RESTART: 1012,
  TRY_LATER: 1013,
  BAD_MESSAGE: 4400,
  SESSION: 4401,
  REVOKED: 4403,
  GONE: 4404,
  TOO_LARGE: 4413,
  TOO_MANY: 4429,
} as const

export type TerminalReason = 'revoked' | 'deleted' | 'too-large' | 'session'

export type CollabStatus =
  | 'connecting'
  | 'live'
  | 'reconnecting'
  /** The server's lineage changed; the caller must drop its document and load the page again. */
  | 'epoch-reset'
  /** The server will not take this connection again (access lost, page deleted, too large, no session). */
  | 'closed'

export type CollabState = {
  status: CollabStatus
  /** Sync with the server has completed on the current connection. */
  synced: boolean
  /** Edits made while no connection was open and not yet exchanged with the server. */
  unsynced: number
  /** Something changed since the last materialization the server announced. */
  saving: boolean
  /** Revision of the last materialization announced, if any. */
  savedRevision: number | null
  closedReason: TerminalReason | null
  /** The last close code seen. */
  lastCloseCode: number | null
  title: string | null
  /** Bumps whenever the server says comments changed. */
  commentsTick: number
}

export type SocketLike = {
  binaryType: string
  readyState: number
  onopen: ((event: unknown) => void) | null
  onmessage: ((event: { data: unknown }) => void) | null
  onclose: ((event: { code: number }) => void) | null
  onerror: ((event: unknown) => void) | null
  send: (data: Uint8Array | string) => void
  close: (code?: number, reason?: string) => void
}

export type CollabOptions = {
  url: string
  doc: Y.Doc
  awareness: awarenessProtocol.Awareness
  /** The epoch of a connection this document came from, if any (a reload keeps it). */
  knownEpoch?: string | null
  createSocket?: (url: string) => SocketLike
  /** First retry delay in ms; doubles up to `maxDelayMs`. */
  baseDelayMs?: number
  maxDelayMs?: number
  random?: () => number
}

const OPEN = 1

function terminalReason(code: number): TerminalReason | null {
  if (code === CLOSE.SESSION) return 'session'
  if (code === CLOSE.REVOKED) return 'revoked'
  if (code === CLOSE.GONE) return 'deleted'
  if (code === CLOSE.TOO_LARGE) return 'too-large'
  return null
}

export class DocCollabProvider {
  readonly doc: Y.Doc
  readonly awareness: awarenessProtocol.Awareness
  private readonly options: CollabOptions
  private socket: SocketLike | null = null
  /** Called before an incoming sync message is applied, while the shared document still equals the editor. */
  beforeRemoteUpdate: (() => void) | null = null
  private state: CollabState = {
    status: 'connecting',
    synced: false,
    unsynced: 0,
    saving: false,
    savedRevision: null,
    closedReason: null,
    lastCloseCode: null,
    title: null,
    commentsTick: 0,
  }
  private listeners = new Set<() => void>()
  private epoch: string | null
  private attempt = 0
  private retryTimer: ReturnType<typeof setTimeout> | null = null
  private heartbeat: ReturnType<typeof setInterval> | null = null
  private destroyed = false
  private everConnected = false

  constructor(options: CollabOptions) {
    this.options = options
    this.doc = options.doc
    this.awareness = options.awareness
    this.epoch = options.knownEpoch ?? null
    this.doc.on('update', this.onDocUpdate)
    this.awareness.on('update', this.onAwarenessUpdate)
    // y-protocols drops a remote cursor that has not renewed within 30 s; renew ours.
    this.heartbeat = setInterval(() => {
      const local = this.awareness.getLocalState()
      if (local !== null) this.awareness.setLocalState(local)
    }, 15_000)
  }

  get lineage(): string | null {
    return this.epoch
  }

  getState = (): CollabState => this.state

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  private set(patch: Partial<CollabState>): void {
    this.state = { ...this.state, ...patch }
    for (const listener of this.listeners) listener()
  }

  connect(): void {
    if (this.destroyed || this.state.status === 'closed' || this.state.status === 'epoch-reset') return
    this.clearRetry()
    this.set({ status: this.everConnected ? 'reconnecting' : 'connecting', synced: false })
    const socket = (this.options.createSocket ?? ((url) => new WebSocket(url) as unknown as SocketLike))(this.options.url)
    socket.binaryType = 'arraybuffer'
    this.socket = socket
    socket.onmessage = (event) => this.onMessage(socket, event.data)
    socket.onclose = (event) => this.onClose(socket, event.code)
    socket.onerror = () => {
      // The close event that follows decides what happens.
    }
  }

  destroy(): void {
    this.destroyed = true
    this.clearRetry()
    if (this.heartbeat) clearInterval(this.heartbeat)
    this.doc.off('update', this.onDocUpdate)
    this.awareness.off('update', this.onAwarenessUpdate)
    const socket = this.socket
    this.socket = null
    if (socket) {
      socket.onmessage = null
      socket.onclose = null
      try {
        socket.close(1000, 'leaving')
      } catch {
        // Already closed.
      }
    }
    awarenessProtocol.removeAwarenessStates(this.awareness, [this.doc.clientID], 'destroy')
    this.listeners.clear()
  }

  private clearRetry(): void {
    if (this.retryTimer) clearTimeout(this.retryTimer)
    this.retryTimer = null
  }

  private send(data: Uint8Array): void {
    if (this.socket && this.socket.readyState === OPEN) this.socket.send(data)
  }

  private onDocUpdate = (update: Uint8Array, origin: unknown): void => {
    // Anything that arrives from the server (origin === this) is already known there.
    if (origin === this) {
      this.set({ saving: true })
      return
    }
    const connected = Boolean(this.socket && this.socket.readyState === OPEN && this.state.synced)
    if (connected) {
      const encoder = encoding.createEncoder()
      encoding.writeVarUint(encoder, MESSAGE_SYNC)
      syncProtocol.writeUpdate(encoder, update)
      this.send(encoding.toUint8Array(encoder))
      this.set({ saving: true })
    } else {
      this.set({ unsynced: this.state.unsynced + 1, saving: true })
    }
  }

  private onAwarenessUpdate = (
    { added, updated, removed }: { added: number[]; updated: number[]; removed: number[] },
    origin: unknown,
  ): void => {
    if (origin === this) return
    const changed = added.concat(updated, removed).filter((id) => id === this.doc.clientID)
    if (changed.length === 0 || !this.socket || this.socket.readyState !== OPEN || !this.state.synced) return
    const encoder = encoding.createEncoder()
    encoding.writeVarUint(encoder, MESSAGE_AWARENESS)
    encoding.writeVarUint8Array(encoder, awarenessProtocol.encodeAwarenessUpdate(this.awareness, changed))
    this.send(encoding.toUint8Array(encoder))
  }

  private onMessage(socket: SocketLike, data: unknown): void {
    if (socket !== this.socket) return
    if (typeof data === 'string') {
      this.onControl(socket, data)
      return
    }
    if (!(data instanceof ArrayBuffer) && !ArrayBuffer.isView(data)) return
    const bytes = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
    const decoder = decoding.createDecoder(bytes)
    const type = decoding.readVarUint(decoder)
    if (type === MESSAGE_SYNC) {
      this.beforeRemoteUpdate?.()
      const encoder = encoding.createEncoder()
      encoding.writeVarUint(encoder, MESSAGE_SYNC)
      const syncType = syncProtocol.readSyncMessage(decoder, encoder, this.doc, this)
      if (encoding.length(encoder) > 1) this.send(encoding.toUint8Array(encoder))
      if (syncType === syncProtocol.messageYjsSyncStep2) this.onSynced()
    } else if (type === MESSAGE_AWARENESS) {
      awarenessProtocol.applyAwarenessUpdate(this.awareness, decoding.readVarUint8Array(decoder), this)
    }
  }

  private onSynced(): void {
    if (this.state.synced) return
    this.set({ synced: true, unsynced: 0, status: 'live' })
  }

  private onControl(socket: SocketLike, text: string): void {
    let frame: { type?: string; epoch?: string; revisionNo?: number; title?: string }
    try {
      frame = JSON.parse(text) as typeof frame
    } catch {
      return
    }
    if (frame.type === 'hello') {
      const epoch = frame.epoch ?? null
      if (this.epoch !== null && epoch !== null && epoch !== this.epoch) {
        // A different lineage: merging this document into it would duplicate the text.
        // The caller discards the document and loads the page again.
        this.set({ status: 'epoch-reset', synced: false })
        this.detach(socket)
        return
      }
      this.epoch = epoch
      this.everConnected = true
      this.attempt = 0
      // The server sent its state vector next; ours goes out now, then our cursor.
      const encoder = encoding.createEncoder()
      encoding.writeVarUint(encoder, MESSAGE_SYNC)
      syncProtocol.writeSyncStep1(encoder, this.doc)
      this.send(encoding.toUint8Array(encoder))
      const local = this.awareness.getLocalState()
      if (local !== null) {
        const aware = encoding.createEncoder()
        encoding.writeVarUint(aware, MESSAGE_AWARENESS)
        encoding.writeVarUint8Array(aware, awarenessProtocol.encodeAwarenessUpdate(this.awareness, [this.doc.clientID]))
        this.send(encoding.toUint8Array(aware))
      }
    } else if (frame.type === 'materialized') {
      this.set({ saving: false, savedRevision: frame.revisionNo ?? this.state.savedRevision })
    } else if (frame.type === 'meta') {
      this.set({ title: frame.title ?? null })
    } else if (frame.type === 'comments_changed') {
      this.set({ commentsTick: this.state.commentsTick + 1 })
    }
    // `revoked` needs no handling of its own: the close code that follows says the same.
  }

  private detach(socket: SocketLike): void {
    socket.onmessage = null
    socket.onclose = null
    try {
      socket.close(1000, 'lineage changed')
    } catch {
      // Already closed.
    }
    if (this.socket === socket) this.socket = null
  }

  private onClose(socket: SocketLike, code: number): void {
    if (socket !== this.socket || this.destroyed) return
    this.socket = null
    // Other people's cursors are gone with the connection.
    const remote = Array.from(this.awareness.getStates().keys()).filter((id) => id !== this.doc.clientID)
    awarenessProtocol.removeAwarenessStates(this.awareness, remote, this)
    const reason = terminalReason(code)
    if (reason) {
      this.set({ status: 'closed', closedReason: reason, synced: false, lastCloseCode: code })
      return
    }
    this.set({ status: this.everConnected ? 'reconnecting' : 'connecting', synced: false, lastCloseCode: code })
    this.scheduleReconnect()
  }

  private scheduleReconnect(): void {
    const base = this.options.baseDelayMs ?? 1000
    const max = this.options.maxDelayMs ?? 30_000
    const random = this.options.random ?? Math.random
    const delay = Math.min(max, base * 2 ** this.attempt)
    this.attempt += 1
    // Up to a quarter more, so a restarted server does not meet every tab at the same instant.
    const jittered = Math.round(delay * (1 + random() * 0.25))
    this.clearRetry()
    this.retryTimer = setTimeout(() => this.connect(), jittered)
  }
}

export function collabUrl(docId: string, location: Pick<Location, 'protocol' | 'host'>): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${location.host}/v1/docs/${docId}/collab`
}
