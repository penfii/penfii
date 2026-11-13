import { useCallback, useEffect, useRef, useState, type UIEvent } from 'react'

/** The scrollers that sit below a secondary header: a board, a list, a conversation. A page that scrolls its own header away needs nothing. */
const SCROLLERS = '.board-viewport, .work-scroll, .scroller'
/** Movement below this many pixels is a tremor, not a direction. */
const STEP = 12
/** Near the top the chrome is always shown. */
const TOP = 24

/**
 * Scroll direction decides whether a screen's secondary header (the project's section tabs and tools,
 * a channel's header) is shown: down hides it, up brings it back. The top app bar and the tabs are
 * never part of this. A new location starts expanded. The scroller must have room to spare for the
 * header it frees, or hiding it would shorten the scroll range and flip the state back and forth.
 */
export function useChromeCollapse(locationKey: string): { collapsed: boolean; onScroll: (event: UIEvent<HTMLElement>) => void } {
  const [collapsed, setCollapsed] = useState(false)
  const last = useRef(new WeakMap<Element, number>())
  useEffect(() => { setCollapsed(false) }, [locationKey])
  const onScroll = useCallback((event: UIEvent<HTMLElement>) => {
    const target = event.target
    if (!(target instanceof HTMLElement) || !target.matches(SCROLLERS)) return
    const top = target.scrollTop
    const before = last.current.get(target)
    if (before === undefined) {
      last.current.set(target, top)
      return
    }
    const delta = top - before
    if (top <= TOP) {
      last.current.set(target, top)
      setCollapsed(false)
    } else if (delta > STEP) {
      last.current.set(target, top)
      // Only a scroller that can still scroll after the header is gone may hide it.
      if (target.scrollHeight - target.clientHeight > 200) setCollapsed(true)
    } else if (delta < -STEP) {
      last.current.set(target, top)
      setCollapsed(false)
    }
  }, [])
  return { collapsed, onScroll }
}

/**
 * The visible part of the window: the phone shell is as tall as this, so with the keyboard open the
 * composer or the editor's toolbar sits right above it instead of behind it (iOS does not shrink the layout
 * viewport for the keyboard, Android may or may not). `keyboard` is true while the keyboard takes a real share.
 */
export function useKeyboardViewport(): boolean {
  const [keyboard, setKeyboard] = useState(false)
  useEffect(() => {
    const viewport = window.visualViewport
    if (!viewport) return
    const root = document.documentElement
    const update = () => {
      const hidden = window.innerHeight - viewport.height - viewport.offsetTop
      root.style.setProperty('--m-vh', `${String(Math.round(viewport.height))}px`)
      setKeyboard(hidden > 120)
      // Focusing a field can scroll the whole page; the shell never scrolls, so put it back.
      if (viewport.offsetTop > 0 && hidden > 120) window.scrollTo(0, 0)
    }
    update()
    viewport.addEventListener('resize', update)
    viewport.addEventListener('scroll', update)
    return () => {
      viewport.removeEventListener('resize', update)
      viewport.removeEventListener('scroll', update)
      root.style.removeProperty('--m-vh')
    }
  }, [])
  return keyboard
}
