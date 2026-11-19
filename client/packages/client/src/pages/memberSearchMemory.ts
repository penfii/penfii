import { useCallback, useEffect, useRef, useState, type MouseEvent } from 'react'
import { useLocation } from 'react-router'

type Memory = { query: string; scroll: number; focus: string | null }

const EMPTY: Memory = { query: '', scroll: 0, focus: null }

function read(key: string): Memory {
  try {
    const raw = sessionStorage.getItem(key)
    if (!raw) return EMPTY
    const value = JSON.parse(raw) as Partial<Memory>
    return { query: typeof value.query === 'string' ? value.query : '', scroll: typeof value.scroll === 'number' ? value.scroll : 0, focus: typeof value.focus === 'string' ? value.focus : null }
  } catch {
    return EMPTY
  }
}

function write(key: string, memory: Memory) {
  try {
    if (memory.query === '' && memory.scroll === 0 && memory.focus === null) sessionStorage.removeItem(key)
    else sessionStorage.setItem(key, JSON.stringify(memory))
  } catch {
    // Without storage the search simply starts empty after Back.
  }
}

/**
 * Where a person was in the member search: the text, how far the list was scrolled and which row was last opened.
 * It is kept per browser history entry (the router's location key) in this tab's session storage, so Back, Forward and a
 * refresh return to the same search, while a new tab, a shared link or another Workspace starts empty. The text is
 * deliberately not put in the address: it may be an email, and an address is shared and logged.
 */
export function useMemberSearchMemory(workspaceId: string, ready: boolean) {
  const { key } = useLocation()
  const storageKey = `penfii.members.search.${workspaceId}.${key}`
  const memory = useRef<Memory>(read(storageKey))
  const [query, setQueryState] = useState(memory.current.query)
  const rootRef = useRef<HTMLDivElement>(null)
  const restored = useRef(false)

  const save = useCallback(
    (patch: Partial<Memory>) => {
      memory.current = { ...memory.current, ...patch }
      write(storageKey, memory.current)
    },
    [storageKey],
  )

  // The same page at another history entry or Workspace is a different search: never carry the text across.
  const lastKey = useRef(storageKey)
  useEffect(() => {
    if (lastKey.current === storageKey) return
    lastKey.current = storageKey
    memory.current = read(storageKey)
    restored.current = false
    setQueryState(memory.current.query)
  }, [storageKey])

  const setQuery = useCallback(
    (value: string) => {
      setQueryState(value)
      // A new search starts from the top; what was remembered about the old one no longer applies.
      save({ query: value, scroll: 0, focus: null })
    },
    [save],
  )

  const frame = useRef(0)
  useEffect(() => () => cancelAnimationFrame(frame.current), [])
  const onScroll = useCallback(() => {
    cancelAnimationFrame(frame.current)
    frame.current = requestAnimationFrame(() => save({ scroll: rootRef.current?.scrollTop ?? 0 }))
  }, [save])

  const onClickCapture = useCallback(
    (event: MouseEvent) => {
      const row = (event.target as HTMLElement).closest<HTMLElement>('[data-person-id]')
      if (row?.dataset.personId) save({ focus: row.dataset.personId })
    },
    [save],
  )

  // Once the list is there, scroll to where it was and put the keyboard back on the row that was last used.
  useEffect(() => {
    if (!ready || restored.current) return
    restored.current = true
    const root = rootRef.current
    if (!root) return
    if (memory.current.scroll > 0) root.scrollTop = memory.current.scroll
    const focus = memory.current.focus
    if (focus) root.querySelector<HTMLElement>(`[data-person-id="${focus.replace(/["\\]/g, "\\$&")}"] .person-button`)?.focus({ preventScroll: true })
  }, [ready])

  return { query, setQuery, rootRef, onScroll, onClickCapture }
}
