import { afterEach, vi } from 'vitest'
import { loadCatalog } from '../i18n/catalog'

// The app fetches the texts of its language before the first render; a test draws at once, so both are in before it starts.
await Promise.all([loadCatalog('ko-KR'), loadCatalog('en-US')])

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

// A query function that returns nothing is a test that forgot to say what the server answers: the screen under test then
// shows its error state and the test can still pass. TanStack reports it with console.error; that now fails the test.
// `console.error` and `console.warn` are otherwise left alone (a test that expects one spies on it).
const unanswered: string[] = []
const report = console.error.bind(console)
console.error = (...args: unknown[]) => {
  const text = args.map((arg) => (typeof arg === 'string' ? arg : arg instanceof Error ? arg.message : '')).join(' ')
  if (/Query data cannot be undefined/.test(text)) unanswered.push(text.replace(/\s+/g, ' ').slice(0, 300))
  report(...args)
}
afterEach(() => {
  const found = unanswered.splice(0)
  if (found.length > 0) {
    throw new Error(`A query got no data from the test's mock:\n${found.join('\n')}\nMake the mock return what the real API returns for this call.`)
  }
})
