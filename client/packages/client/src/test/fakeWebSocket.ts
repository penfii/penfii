import { vi } from 'vitest'

/**
 * A WebSocket a test drives by hand: it never connects by itself. `open()` is the server accepting, `receive()` a frame
 * from the server, `drop()` the connection ending. What the client sent is in `sent`.
 * `installFakeWebSocket()` replaces the global for the test (and returns every socket opened, oldest first).
 */
export class FakeWebSocket extends EventTarget {
  static readonly instances: FakeWebSocket[] = []
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readyState = FakeWebSocket.CONNECTING
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  readonly sent: unknown[] = []

  constructor(readonly url: string) {
    super()
    FakeWebSocket.instances.push(this)
  }

  send(data: string): void {
    this.sent.push(JSON.parse(data) as unknown)
  }

  /** The client closing it (unmount, session end). */
  close(): void {
    if (this.readyState === FakeWebSocket.CLOSED) return
    this.readyState = FakeWebSocket.CLOSED
    this.onclose?.(new Event('close') as CloseEvent)
  }

  open(): void {
    this.readyState = FakeWebSocket.OPEN
    this.onopen?.(new Event('open'))
  }

  receive(frame: unknown): void {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(frame) }))
  }

  /** The connection ends without the client asking (the server stopped, the network dropped). */
  drop(): void {
    this.close()
  }
}

export function installFakeWebSocket(): FakeWebSocket[] {
  FakeWebSocket.instances.length = 0
  vi.stubGlobal('WebSocket', FakeWebSocket)
  return FakeWebSocket.instances
}
