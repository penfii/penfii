import type { ReferenceKind, ResolvedReference } from '@penfii/contracts'

type Key = `${ReferenceKind}:${string}`
const keyOf = (kind: ReferenceKind, id: string): Key => `${kind}:${id}`

/**
 * What a reference or mention node shows is never stored in the page: the editor asks the server
 * (`references/resolve`) and gets a title only for what the viewer may read. Requests made in the same
 * moment go out as one call (at most 100 items), and an answer is kept for the life of the page view.
 */
export class ReferenceResolver {
  private cache = new Map<Key, ResolvedReference>()
  private waiting = new Map<Key, { kind: ReferenceKind; id: string }>()
  private inflight = new Set<Key>()
  private timer: ReturnType<typeof setTimeout> | null = null
  private listeners = new Set<() => void>()
  private version = 0

  constructor(
    private readonly fetchAll: (items: Array<{ kind: ReferenceKind; id: string }>) => Promise<ResolvedReference[]>,
    private readonly delayMs = 25,
  ) {}

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  /** Changes whenever an answer arrives, for `useSyncExternalStore`. */
  getVersion = (): number => this.version

  peek(kind: ReferenceKind, id: string): ResolvedReference | undefined {
    return this.cache.get(keyOf(kind, id))
  }

  /** Returns what is known now and makes sure the rest is asked for. */
  want(kind: ReferenceKind, id: string): ResolvedReference | undefined {
    const key = keyOf(kind, id)
    const known = this.cache.get(key)
    if (known || this.inflight.has(key) || this.waiting.has(key)) return known
    this.waiting.set(key, { kind, id })
    if (!this.timer) this.timer = setTimeout(() => void this.flush(), this.delayMs)
    return undefined
  }

  /** Forgets every answer (permissions may have changed) and asks again for what is on screen. */
  refresh(): void {
    const again = Array.from(this.cache.values()).map((r) => ({ kind: r.kind, id: r.id }))
    this.cache.clear()
    for (const item of again) this.want(item.kind, item.id)
  }

  private async flush(): Promise<void> {
    this.timer = null
    const batch = Array.from(this.waiting.entries()).slice(0, 100)
    for (const [key] of batch) {
      this.waiting.delete(key)
      this.inflight.add(key)
    }
    if (batch.length === 0) return
    try {
      const answers = await this.fetchAll(batch.map(([, item]) => item))
      for (const answer of answers) this.cache.set(keyOf(answer.kind, answer.id), answer)
    } catch {
      // Unanswered items render as unavailable; asking again happens on the next refresh.
      for (const [key, item] of batch) {
        if (!this.cache.has(key)) this.cache.set(key, { kind: item.kind, id: item.id, available: false })
      }
    } finally {
      for (const [key] of batch) this.inflight.delete(key)
      this.version += 1
      for (const listener of this.listeners) listener()
    }
    if (this.waiting.size > 0 && !this.timer) this.timer = setTimeout(() => void this.flush(), this.delayMs)
  }
}
