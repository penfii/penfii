export const RAIL_WIDTH = 76
export const DETAIL_WIDTH = 320
/** Matches the stylesheet breakpoint where the shell stacks and the thread overlays the chat. */
export const STACKED_MAX_WIDTH = 960
/**
 * Beside a thread the main chat column keeps at least this much (the stylesheet's `minmax(640px, 1fr)`). A thread too wide
 * for that scrolls the conversation sideways instead of squeezing the main column.
 */
export const MAIN_MIN = 640
/**
 * Part of the work area that stays in view beside a side pane at its widest (thread, Ticket panel, context nav).
 * With the thread that is the visible part of the 640px main column; the rest is reached by scrolling sideways.
 */
export const MAIN_VISIBLE_MIN = 320
/**
 * The chat (or board) column keeps this much before the context nav is put away.
 * When a thread or ticket panel is open and the main column would fall under 720,
 * the nav collapses first and the rail stays.
 */
export const CHAT_COMFORT = 720

/** The strip the collapsed nav's "Show sidebar" tab keeps beside the rail (`--nav-edge`). */
export const NAV_EDGE_WIDTH = 24
/** What the context nav takes from the work area: its width, or the tab's strip while a side pane has put it away. */
export function navTrackWidth(navWidth: number, collapsed: boolean): number {
  return collapsed ? NAV_EDGE_WIDTH : navWidth
}

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
/** Narrowest the thread opens at by default: its header, messages, composer (Bold, emoji, attach, Send) and handle still fit. */
export const THREAD_FIT_MIN = 280

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

const DOUBLE_CLICK_MS = 500

/**
 * True when a press on a resize handle follows the previous one within half a second (it restores the default width).
 * `previous` is NEGATIVE_INFINITY before the first press; event time stamps count from page load, so 0 would not do.
 */
export function isSecondPress(previous: number, now: number): boolean {
  const gap = now - previous
  return gap > 0 && gap < DOUBLE_CLICK_MS
}

/**
 * Width a thread opens at while the person has not set one: {@link THREAD_DEFAULT}, or less when the window leaves the
 * main column under {@link MAIN_MIN} beside it, so a narrow desktop does not open with a sideways scroll. Never under
 * {@link THREAD_FIT_MIN}; below that the main column scrolls sideways as for a wide thread.
 */
export function fittedThreadDefault(viewport: number, navWidth: number, detailWidth: number): number {
  const fit = viewport - RAIL_WIDTH - navWidth - MAIN_MIN - detailWidth
  return Math.min(THREAD_DEFAULT, Math.max(THREAD_FIT_MIN, fit))
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

/** Forgets the width under `key`, so the pane goes back to its default (and, for the thread, its fitted default). */
export function clearStoredWidth(key: string): void {
  try {
    localStorage.removeItem(key)
  } catch {
    // As in writeStoredWidth: the in-memory value still applies.
  }
}

export function writeStoredWidth(key: string, value: number): void {
  try {
    localStorage.setItem(key, String(Math.round(value)))
  } catch {
    // Width is a local preference. The current layout still uses the in-memory value.
  }
}

/**
 * Width the Detail Panel takes from the shell's grid, by the `panel` search parameter: a Ticket panel its own width above
 * the overlay breakpoint (below it the panel floats over the page), any other panel {@link DETAIL_WIDTH}, none 0.
 */
export function detailColumnWidth(viewport: number, panel: string | null, ticketWidth: number): number {
  if (!panel) return 0
  if (panel.startsWith('ticket')) return viewport > OVERLAY_MAX_WIDTH ? ticketWidth : 0
  return DETAIL_WIDTH
}

export function maxContextNav(viewport: number, threadWidth: number, detailWidth: number): number {
  return viewport - RAIL_WIDTH - MAIN_VISIBLE_MIN - threadWidth - detailWidth
}

/** The thread may grow until only {@link MAIN_VISIBLE_MIN} of the work area beside it is left in view. */
export function maxThreadWidth(viewport: number, navWidth: number, detailWidth: number): number {
  return viewport - RAIL_WIDTH - navWidth - MAIN_VISIBLE_MIN - detailWidth
}

/**
 * Room for a Ticket panel, by the thread's rule: everything the rail, the sidebar and the part of the
 * work area kept in view (`MAIN_VISIBLE_MIN`) leave. The Board scrolls sideways inside that column instead of squeezing
 * its columns. Below the overlay breakpoint the panel floats above the page, so only the rail stays.
 */
export function maxTicketWidth(viewport: number, navWidth: number): number {
  if (viewport <= STACKED_MAX_WIDTH) return viewport
  if (viewport <= OVERLAY_MAX_WIDTH) return viewport - RAIL_WIDTH
  return viewport - RAIL_WIDTH - navWidth - MAIN_VISIBLE_MIN
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
