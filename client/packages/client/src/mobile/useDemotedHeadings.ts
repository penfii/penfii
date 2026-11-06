import { useEffect, type RefObject } from 'react'

/**
 * A phone screen has exactly one level-1 heading: the title of the push bar, or a hidden one on a root. The pages
 * inside bring their own `<h1>` for the desktop, so here they are announced one level down (`aria-level`), whenever
 * they appear. The visual size is untouched.
 */
export function useDemotedHeadings(container: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    const root = container.current
    if (!root) return
    const demote = () => root.querySelectorAll('h1:not([aria-level])').forEach((heading) => heading.setAttribute('aria-level', '2'))
    demote()
    const observer = new MutationObserver(demote)
    observer.observe(root, { childList: true, subtree: true })
    return () => observer.disconnect()
  }, [container])
}
