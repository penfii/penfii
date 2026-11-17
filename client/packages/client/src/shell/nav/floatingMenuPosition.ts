export type Rect = { top: number; right: number; bottom: number; left: number; width: number; height: number }

/** Fixed-position menu coordinates kept inside the viewport, preferring the side with more room. */
export function floatingMenuPosition(anchor: Rect, popup: Pick<Rect, 'width' | 'height'>, viewport: { width: number; height: number }, margin = 8, gap = 4) {
  const maxHeight = Math.max(0, viewport.height - margin * 2)
  const maxWidth = Math.max(0, viewport.width - margin * 2)
  const width = Math.min(popup.width, maxWidth)
  const below = viewport.height - anchor.bottom - gap - margin
  const above = anchor.top - gap - margin
  const openBelow = popup.height <= below || below >= above
  const height = Math.min(popup.height, maxHeight)
  // Kept inside on both sides, also when the trigger itself has left the viewport (the list scrolled, the window
  // shrank while the menu was open).
  const wanted = openBelow ? anchor.bottom + gap : anchor.top - gap - height
  const top = Math.max(margin, Math.min(wanted, viewport.height - margin - height))
  const left = Math.max(margin, Math.min(anchor.right - width, viewport.width - width - margin))
  return { top, left, maxHeight, maxWidth }
}
