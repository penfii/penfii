import type { PointerEvent as ReactPointerEvent } from 'react'

/** Width state and drag handler of a resizable pane (see `usePaneWidth`). */
export type PaneControls = {
  width: number
  min: number
  max: number
  beginDrag: (event: ReactPointerEvent<HTMLElement>, direction: 1 | -1) => void
}

/**
 * The drag handle on the leading edge of a side pane, shared by the Chat thread and the Ticket
 * panels so they are grabbed and dragged the same way. A pane that does not clip its children
 * also gets the part of the grab zone that lies outside its edge.
 */
export function ResizeHandle({ pane, label, resetLabel }: { pane: PaneControls; label: string; resetLabel?: string }) {
  return (
    <div
      className="resize-handle start"
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      title={resetLabel}
      aria-valuemin={pane.min}
      aria-valuemax={pane.max}
      aria-valuenow={pane.width}
      onPointerDown={(event) => pane.beginDrag(event, -1)}
    />
  )
}
