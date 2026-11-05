import { useLayoutEffect, useRef, useState } from 'react'

const NEAR_BOTTOM = 80

function distanceFromBottom(element: HTMLElement) {
  return element.scrollHeight - element.scrollTop - element.clientHeight
}

export function revealMessage(scroller: HTMLElement, id: string) {
  const node = scroller.querySelector(`[data-message-id="${CSS.escape(id)}"]`)
  if (!(node instanceof HTMLElement)) return
  const view = scroller.getBoundingClientRect()
  const box = node.getBoundingClientRect()
  const pad = 12
  const fullyVisible = box.top >= view.top + pad && box.bottom <= view.bottom - pad
  if (!fullyVisible) {
    const top = box.top - view.top + scroller.scrollTop
    if (box.height >= scroller.clientHeight - pad * 2 || box.top < view.top + pad) {
      scroller.scrollTop = Math.max(0, top - pad)
    } else {
      scroller.scrollTop = Math.max(0, top - (scroller.clientHeight - box.height - pad))
    }
  }
  node.classList.add('flash')
  window.setTimeout(() => node.classList.remove('flash'), 1400)
}

/** Keeps a message list on the latest row only while the reader is already there. */
export function useFollowScroll(scroller: HTMLElement | null, version: number, resetKey: string) {
  const atBottom = useRef(true)
  const stick = useRef(true)
  const seen = useRef(0)
  const hold = useRef<{ height: number; top: number } | null>(null)
  const reset = useRef(resetKey)
  const [pending, setPending] = useState(0)

  useLayoutEffect(() => {
    if (!scroller) return
    const onScroll = () => {
      const near = distanceFromBottom(scroller) <= NEAR_BOTTOM
      atBottom.current = near
      if (near) setPending(0)
    }
    scroller.addEventListener('scroll', onScroll, { passive: true })
    return () => scroller.removeEventListener('scroll', onScroll)
  }, [scroller])

  useLayoutEffect(() => {
    if (!scroller) return
    if (reset.current !== resetKey) {
      reset.current = resetKey
      seen.current = version
      stick.current = false
      atBottom.current = true
      scroller.scrollTop = scroller.scrollHeight
      setPending(0)
      return
    }
    const delta = version - seen.current
    seen.current = version
    if (hold.current) {
      const added = scroller.scrollHeight - hold.current.height
      scroller.scrollTop = hold.current.top + Math.max(0, added)
      hold.current = null
      return
    }
    if (stick.current || (delta > 0 && atBottom.current)) {
      scroller.scrollTop = scroller.scrollHeight
      stick.current = false
      atBottom.current = true
      setPending(0)
      return
    }
    if (delta <= 0) return
    setPending((count) => count + delta)
  }, [resetKey, scroller, version])

  return {
    pending,
    pin() {
      stick.current = true
    },
    holdScroll() {
      if (!scroller) return
      hold.current = { height: scroller.scrollHeight, top: scroller.scrollTop }
    },
    releaseHold() {
      hold.current = null
    },
    jump() {
      if (!scroller) return
      scroller.scrollTop = scroller.scrollHeight
      atBottom.current = true
      stick.current = false
      setPending(0)
    },
  }
}
