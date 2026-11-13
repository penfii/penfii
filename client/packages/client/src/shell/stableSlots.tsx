import { useLayoutEffect, useRef, useState } from 'react'

/**
 * The page and the Detail Panel are rendered once, by `ResponsiveShell`, into elements it owns. The desktop and the
 * phone frame only say where those elements go (`HostSlot`). A resize across the phone width swaps the frame and moves
 * the elements; the React tree inside them is the same one, so what was typed in a form or an editor is still there.
 */
export type ShellSlots = { page: HTMLElement; detail: HTMLElement }

/** An element that lives for as long as the shell does and is moved between frames. */
export function useSlotElements(): ShellSlots {
  const [slots] = useState<ShellSlots>(() => {
    const page = document.createElement('div')
    page.className = 'product-main'
    // Its children take part in the frame's layout as if the element were not there.
    const detail = document.createElement('div')
    detail.style.display = 'contents'
    return { page, detail }
  })
  return slots
}

/**
 * Puts `host` into the frame, right after this (invisible) marker. React owns the marker; the host is moved by hand, and
 * is taken out again before the frame is removed so that it survives into the next frame.
 */
export function HostSlot({ host }: { host: HTMLElement }) {
  const marker = useRef<HTMLSpanElement>(null)
  useLayoutEffect(() => {
    const at = marker.current
    if (!at) return
    at.after(host)
    return () => {
      host.remove()
    }
  }, [host])
  return <span ref={marker} hidden data-host-slot="" />
}
