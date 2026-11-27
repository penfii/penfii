import { describe, expect, it } from 'vitest'
import { TICKET_MIN, maxTicketWidth, MAIN_MIN, MAIN_VISIBLE_MIN, RAIL_WIDTH, clampWidth, dragSample, maxContextNav, maxThreadWidth, NAV_DEFAULT, NAV_MIN, THREAD_DEFAULT, THREAD_MIN, detailColumnWidth, fittedThreadDefault, isSecondPress, NAV_EDGE_WIDTH, navTrackWidth, paneRoom, shouldCollapseNav, THREAD_FIT_MIN } from './paneWidth'

describe('pane widths', () => {
  it('keeps a pane inside the room the window leaves', () => {
    expect(clampWidth(80, NAV_MIN, 900)).toBe(NAV_MIN)
    expect(clampWidth(900, NAV_MIN, 700)).toBe(700)
    expect(clampWidth(280, NAV_MIN, 900)).toBe(280)
  })

  it('lets the thread grow wider than the main column', () => {
    const viewport = 1280
    const nav = 300
    const thread = 560
    const room = maxThreadWidth(viewport, nav, 0)
    expect(room).toBe(viewport - RAIL_WIDTH - nav - MAIN_VISIBLE_MIN)
    expect(clampWidth(thread, THREAD_MIN, room)).toBe(560)
    const main = viewport - RAIL_WIDTH - nav - thread
    expect(main).toBeGreaterThanOrEqual(MAIN_VISIBLE_MIN)
    expect(main).toBeLessThan(thread)
  })

  it('lets the thread pass the 640px main column, keeping 320px of the work area in view', () => {
    expect(MAIN_MIN).toBe(640)
    expect(MAIN_VISIBLE_MIN).toBe(320)
    // 1920 with the nav put away: the work area is 1844, so the thread can reach 1524 and the main column scrolls.
    const room = maxThreadWidth(1920, 0, 0)
    expect(room).toBe(1920 - RAIL_WIDTH - 320)
    const thread = clampWidth(1300, THREAD_MIN, room)
    expect(thread).toBe(1300)
    const overflow = MAIN_MIN + thread - (1920 - RAIL_WIDTH)
    expect(overflow).toBe(96)
  })

  it('lets the context nav grow past the old fixed cap when the main column can shrink', () => {
    const viewport = 1280
    const thread = 360
    const room = maxContextNav(viewport, thread, 0)
    expect(room).toBe(viewport - RAIL_WIDTH - MAIN_VISIBLE_MIN - thread)
    expect(room).toBeGreaterThan(400)
    expect(clampWidth(700, NAV_MIN, room)).toBe(Math.min(700, room))
  })

  it('keeps a stacked thread at its saved width instead of reserving a side-by-side main column', () => {
    expect(paneRoom('thread', 900, 384, 0)).toBe(900)
    expect(clampWidth(540, THREAD_MIN, paneRoom('thread', 900, 384, 0))).toBe(540)
    expect(paneRoom('thread', 1280, 384, 0)).toBe(1280 - RAIL_WIDTH - 384 - MAIN_VISIBLE_MIN)
  })

  it('shrinks below the preferred minimum when the window cannot fit every minimum', () => {
    expect(clampWidth(400, THREAD_MIN, 140)).toBe(140)
  })
})

describe('ticket panel width', () => {
  it('follows the thread: same smallest width and the same room rule', () => {
    expect(TICKET_MIN).toBe(THREAD_MIN)
    expect(maxTicketWidth(1440, 248)).toBe(maxThreadWidth(1440, 248, 0))
    expect(maxTicketWidth(1280, 300)).toBe(1280 - RAIL_WIDTH - 300 - MAIN_VISIBLE_MIN)
    expect(maxTicketWidth(1920, 248)).toBeGreaterThan(1200)
    expect(clampWidth(40, TICKET_MIN, maxTicketWidth(1440, 248))).toBe(TICKET_MIN)
    expect(clampWidth(180, TICKET_MIN, maxTicketWidth(1440, 248))).toBe(180)
    expect(clampWidth(5000, TICKET_MIN, maxTicketWidth(1440, 248))).toBe(796)
  })

  it('puts the context nav away before the main column drops under 720', () => {
    expect(shouldCollapseNav(1920, NAV_DEFAULT, 420)).toBe(false)
    expect(shouldCollapseNav(1440, NAV_DEFAULT, 420)).toBe(true)
    expect(shouldCollapseNav(1440, NAV_DEFAULT, 0)).toBe(false)
    expect(shouldCollapseNav(900, NAV_DEFAULT, 420)).toBe(false)
  })

  it('closes once the drag passes the close control, and a double-click width is the default', () => {
    expect(THREAD_MIN).toBe(28 + 16 + 16 + 1)
    const room = maxTicketWidth(1440, 248)
    expect(dragSample(480, -20, THREAD_MIN, room)).toEqual({ width: 460, close: false })
    expect(dragSample(480, -500, THREAD_MIN, room)).toEqual({ width: 480, close: true })
    expect(dragSample(THREAD_MIN, -1, THREAD_MIN, room).close).toBe(true)
    expect(clampWidth(THREAD_DEFAULT, THREAD_MIN, room)).toBe(THREAD_DEFAULT)
  })

  it('overlays on narrow windows and fills the screen on phones', () => {
    expect(maxTicketWidth(1100, 248)).toBe(1100 - RAIL_WIDTH)
    expect(maxTicketWidth(390, 248)).toBe(390)
  })
})

describe('double press on a resize handle', () => {
  it('treats the first press as a drag even right after the page loaded, and a second one within 500ms as a reset', () => {
    expect(isSecondPress(Number.NEGATIVE_INFINITY, 120)).toBe(false)
    expect(isSecondPress(120, 400)).toBe(true)
    expect(isSecondPress(120, 700)).toBe(false)
    expect(isSecondPress(400, 400)).toBe(false)
  })
})

describe('detail column', () => {
  it('is the Ticket panel\'s own width while it takes a grid column, 320 for another panel, and nothing for a floating or absent one', () => {
    expect(detailColumnWidth(1440, 'ticket:abc', 902)).toBe(902)
    expect(detailColumnWidth(1440, 'ticket-new', 480)).toBe(480)
    expect(detailColumnWidth(1180, 'ticket:abc', 902)).toBe(0)
    expect(detailColumnWidth(1440, 'member:abc', 902)).toBe(320)
    expect(detailColumnWidth(1440, null, 902)).toBe(0)
    // A thread beside a 902px Ticket panel at 1440 (nav put away) may grow only until 320px of the work area is left in view.
    expect(maxThreadWidth(1440, NAV_EDGE_WIDTH, detailColumnWidth(1440, 'ticket:abc', 902))).toBe(1440 - RAIL_WIDTH - NAV_EDGE_WIDTH - 902 - 320)
  })
})

describe('the thread\'s first width', () => {
  // With the nav put away, its "Show sidebar" strip stays beside the rail.
  const fitted = (viewport: number, detail = 0) => fittedThreadDefault(viewport, navTrackWidth(NAV_DEFAULT, true), detail)

  it('is 420 where the main column keeps 640 beside it, and only as much as leaves 640 where the window is narrower', () => {
    expect(navTrackWidth(NAV_DEFAULT, true)).toBe(NAV_EDGE_WIDTH)
    expect(navTrackWidth(NAV_DEFAULT, false)).toBe(NAV_DEFAULT)
    expect(fitted(1920)).toBe(THREAD_DEFAULT)
    expect(fitted(1180)).toBe(THREAD_DEFAULT)
    expect(fitted(1100)).toBe(1100 - RAIL_WIDTH - NAV_EDGE_WIDTH - MAIN_MIN)
    expect(fitted(1024)).toBe(284)
  })

  it('never opens narrower than 280; below that the main column scrolls sideways as for a wider thread', () => {
    expect(THREAD_FIT_MIN).toBe(280)
    expect(fitted(1020)).toBe(280)
    expect(fitted(961)).toBe(THREAD_FIT_MIN)
  })

  it('counts an open Detail Panel', () => {
    expect(fitted(1440, 480)).toBe(THREAD_FIT_MIN)
    expect(fitted(1500, 480)).toBe(1500 - RAIL_WIDTH - NAV_EDGE_WIDTH - MAIN_MIN - 480)
    expect(fitted(1920, 480)).toBe(THREAD_DEFAULT)
  })
})
