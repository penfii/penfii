import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from 'react'

/** Width state and drag handler of a resizable pane (see `usePaneWidth`). */
export type PaneControls = {
  width: number
  min: number
  max: number
  beginDrag: (event: ReactPointerEvent<HTMLElement>, direction: 1 | -1) => void
  /** Makes the pane `delta` px wider (negative: narrower) from the keyboard. */
  nudge: (delta: number) => void
}

/** Arrow keys move the edge by this much; with Shift, by the larger step. */
const KEY_STEP = 16
const KEY_STEP_LARGE = 64

/**
 * The drag handle on the leading edge of a side pane, shared by the Chat thread and the Ticket
 * panels so they are grabbed and dragged the same way. A pane that does not clip its children
 * also gets the part of the grab zone that lies outside its edge.
 * The handle can also take focus: Left and Right move the edge (Shift for larger steps). Taking focus scrolls it
 * into view when the conversation scrolls sideways.
 */
export function ResizeHandle({ pane, label, resetLabel }: { pane: PaneControls; label: string; resetLabel?: string }) {
  function onKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
    event.preventDefault()
    const step = event.shiftKey ? KEY_STEP_LARGE : KEY_STEP
    // The handle is on the pane's left edge: moving it left makes the pane wider.
    pane.nudge(event.key === 'ArrowLeft' ? step : -step)
  }
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
      tabIndex={0}
      onKeyDown={onKeyDown}
      onPointerDown={(event) => pane.beginDrag(event, -1)}
    />
  )
}
