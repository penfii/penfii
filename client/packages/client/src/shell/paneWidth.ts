export const RAIL_WIDTH = 76
export const DETAIL_WIDTH = 320
/** Matches the stylesheet breakpoint where the shell stacks and the thread overlays the chat. */
export const STACKED_MAX_WIDTH = 960
/** Smallest main column that still leaves the composer usable. Panes may grow wider than this. */
export const MAIN_MIN = 280
/**
 * The chat (or board) column keeps this much before the context nav is put away.
 * OpenPencil 9.1: when a thread or ticket panel is open and the main column would fall under 720,
 * the nav collapses first and the rail stays.
 */
export const CHAT_COMFORT = 720

export const NAV_MIN = 160
export const NAV_DEFAULT = 248
export const NAV_WIDTH_KEY = 'penfii.contextNavWidth'

/**
 * Smallest width that still shows a detail panel's close control.
 * The control is the 28px icon button, the header pads 16px (`--space-4`) on each side,
 * and the panel edge is 1px. Narrower than this, the panel closes instead of hiding the control.
 */
export const CLOSE_BUTTON_PX = 28
export const PANE_HEADER_PAD_PX = 16
export const PANE_BORDER_PX = 1
const CLOSE_FLOOR = CLOSE_BUTTON_PX + PANE_HEADER_PAD_PX * 2 + PANE_BORDER_PX

export const THREAD_MIN = CLOSE_FLOOR
export const THREAD_DEFAULT = 420
export const THREAD_WIDTH_KEY = 'penfii.threadWidth'

/** The Ticket panels (create and detail) resize exactly like the thread: same smallest width, same room rule. @alias */
export const TICKET_MIN = THREAD_MIN
export const TICKET_DEFAULT = 480
export const TICKET_WIDTH_KEY = 'penfii.ticketPanelWidth'
/** Below this the Ticket panels overlay the page instead of taking a grid column. */
export const OVERLAY_MAX_WIDTH = 1180

/**
 * One sample of a horizontal drag. `delta` is already signed for the handle's edge.
 * Crossing below `floor` closes the pane; the width from before the gesture is kept.
 */
export function dragSample(start: number, delta: number, floor: number, max: number): { width: number; close: boolean } {
  const raw = start + delta
  if (raw < floor) return { width: start, close: true }
  return { width: clampWidth(raw, floor, max), close: false }
}

export function clampWidth(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min
  if (max < min) return Math.max(120, Math.round(max))
  return Math.min(max, Math.max(min, Math.round(value)))
}

export function readStoredWidth(key: string, fallback: number, min: number): number {
  try {
    const parsed = Number(localStorage.getItem(key))
    if (!Number.isFinite(parsed) || parsed < min) return fallback
    return Math.round(parsed)
  } catch {
    return fallback
  }
}

export function writeStoredWidth(key: string, value: number): void {
  try {
    localStorage.setItem(key, String(Math.round(value)))
  } catch {
    // Width is a local preference. The current layout still uses the in-memory value.
  }
}

export function maxContextNav(viewport: number, threadWidth: number, detailWidth: number): number {
  return viewport - RAIL_WIDTH - MAIN_MIN - threadWidth - detailWidth
}

export function maxThreadWidth(viewport: number, navWidth: number, detailWidth: number): number {
  return viewport - RAIL_WIDTH - navWidth - MAIN_MIN - detailWidth
}

/**
 * Room for a Ticket panel, by the thread's rule: everything the rail, the sidebar and the smallest
 * main column (`MAIN_MIN`) leave. The Board scrolls sideways inside that column instead of squeezing
 * its columns. Below the overlay breakpoint the panel floats above the page, so only the rail stays.
 */
export function maxTicketWidth(viewport: number, navWidth: number): number {
  if (viewport <= STACKED_MAX_WIDTH) return viewport
  if (viewport <= OVERLAY_MAX_WIDTH) return viewport - RAIL_WIDTH
  return viewport - RAIL_WIDTH - navWidth - MAIN_MIN
}

/** True when keeping the nav beside an open side pane would leave the main column under {@link CHAT_COMFORT}. */
export function shouldCollapseNav(viewport: number, navWidth: number, sideWidth: number): boolean {
  if (viewport <= STACKED_MAX_WIDTH || sideWidth <= 0) return false
  return viewport - RAIL_WIDTH - navWidth - sideWidth < CHAT_COMFORT
}

export function paneRoom(kind: 'nav' | 'thread', viewport: number, otherWidth: number, detailWidth: number): number {
  if (viewport <= STACKED_MAX_WIDTH) return viewport
  return kind === 'nav'
    ? maxContextNav(viewport, otherWidth, detailWidth)
    : maxThreadWidth(viewport, otherWidth, detailWidth)
}
