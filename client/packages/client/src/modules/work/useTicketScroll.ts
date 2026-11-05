import { useEffect, useRef, useState, type RefObject } from 'react'

const positions = new Map<string, number>()
const MAX_REMEMBERED = 50

function isScroller(node: EventTarget | null): node is HTMLElement {
  return node instanceof HTMLElement && (node.classList.contains('ticket-scroll') || node.classList.contains('ticket-main'))
}

/**
 * Follows whichever area of the Ticket panel scrolls (the whole body when narrow, the main column when wide):
 * edge shadows, whether the compact header and "back to top" apply, and which section is in view. The
 * position is remembered per Ticket so going back returns to it; a Ticket opened for the first time starts at the top.
 */
export function useTicketScroll(root: RefObject<HTMLElement | null>, ticketId: string, sections: string[]) {
  const [scrolled, setScrolled] = useState(false)
  const [titleHidden, setTitleHidden] = useState(false)
  const [active, setActive] = useState<string | null>(null)
  const sectionKey = sections.join('|')
  const scroller = useRef<HTMLElement | null>(null)

  useEffect(() => {
    const panel = root.current?.closest('.ticket-panel')
    if (!panel) return undefined
    const ids = sectionKey.split('|').filter(Boolean)

    function update(el: HTMLElement) {
      scroller.current = el
      el.dataset.edgeTop = String(el.scrollTop > 2)
      el.dataset.edgeBottom = String(el.scrollTop + el.clientHeight < el.scrollHeight - 2)
      setScrolled(el.scrollTop > 240)
      const title = panel?.querySelector('.ticket-title-block')
      const top = el.getBoundingClientRect().top
      // When the main column scrolls beside a fixed title block (a wide panel), the title never leaves the screen.
      setTitleHidden(title && !el.classList.contains('ticket-main') ? title.getBoundingClientRect().bottom < top + 4 : false)
      let current: string | null = null
      for (const id of ids) {
        const node = document.getElementById(id)
        if (node && node.getBoundingClientRect().top <= top + 80) current = id
      }
      setActive(current)
    }

    function onScroll(event: Event) {
      if (!isScroller(event.target)) return
      positions.set(ticketId, event.target.scrollTop)
      if (positions.size > MAX_REMEMBERED) positions.delete(positions.keys().next().value as string)
      update(event.target)
    }

    panel.addEventListener('scroll', onScroll, true)
    const saved = positions.get(ticketId)
    const frame = window.requestAnimationFrame(() => {
      const el = panel.querySelector<HTMLElement>('.ticket-scroll, .ticket-main')
      const candidates = Array.from(panel.querySelectorAll<HTMLElement>('.ticket-scroll, .ticket-main'))
      const target = candidates.find((node) => node.scrollHeight > node.clientHeight) ?? el
      if (target) {
        if (saved) target.scrollTop = saved
        update(target)
      }
    })
    return () => {
      panel.removeEventListener('scroll', onScroll, true)
      window.cancelAnimationFrame(frame)
    }
  }, [root, ticketId, sectionKey])

  function toTop() {
    scroller.current?.scrollTo?.({ top: 0, behavior: 'smooth' })
  }

  return { scrolled, titleHidden, active, toTop }
}
