import { useEffect, useRef } from 'react'

export function useShown(
  root: HTMLElement | null,
  seq: number,
  onVisible: (seq: number, shown: boolean) => void,
) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const element = ref.current
    if (!element || !root || typeof IntersectionObserver !== 'function') return
    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries[0]
        const shown = Boolean(entry && entry.isIntersecting && entry.intersectionRatio >= 0.5)
        onVisible(seq, shown)
      },
      { root, threshold: [0, 0.5, 1] },
    )
    observer.observe(element)
    return () => {
      observer.disconnect()
      onVisible(seq, false)
    }
  }, [onVisible, root, seq])
  return ref
}
