import { vi } from 'vitest'

/**
 * Tests never open a real socket. The runtime's WebSocket performs its handshake through fetch, which on
 * Node 22 (the CI image) rejects with "invalid onError method" under jsdom and fails the run.
 * A socket that never connects is what a Board without a server looks like.
 */
class OfflineWebSocket extends EventTarget {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readyState = OfflineWebSocket.CLOSED
  onopen: ((event: Event) => void) | null = null
  onmessage: ((event: MessageEvent) => void) | null = null
  onclose: ((event: CloseEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  constructor(readonly url: string) {
    super()
  }
  send() {}
  close() {}
}
vi.stubGlobal('WebSocket', OfflineWebSocket)
