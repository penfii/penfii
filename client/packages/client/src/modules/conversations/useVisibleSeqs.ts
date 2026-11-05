import { useCallback, useRef, useState } from 'react'

export function useVisibleSeqs() {
  const map = useRef(new Map<number, boolean>())
  const [seqs, setSeqs] = useState<number[]>([])
  const setVisible = useCallback((seq: number, visible: boolean) => {
    const previous = map.current.get(seq) ?? false
    if (previous === visible) return
    map.current.set(seq, visible)
    setSeqs([...map.current.entries()].flatMap(([value, shown]) => (shown ? [value] : [])))
  }, [])
  return { seqs, setVisible }
}
