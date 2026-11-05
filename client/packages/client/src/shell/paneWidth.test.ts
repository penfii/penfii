import { describe, expect, it } from 'vitest'
import { TICKET_MIN, maxTicketWidth, MAIN_MIN, RAIL_WIDTH, clampWidth, dragSample, maxContextNav, maxThreadWidth, NAV_DEFAULT, NAV_MIN, THREAD_DEFAULT, THREAD_MIN, paneRoom, shouldCollapseNav } from './paneWidth'

describe('pane widths', () => {
  it('keeps a pane inside the room the window leaves', () => {
    expect(clampWidth(80, NAV_MIN, 900)).toBe(NAV_MIN)
    expect(clampWidth(900, NAV_MIN, 700)).toBe(700)
    expect(clampWidth(280, NAV_MIN, 900)).toBe(280)
  })

  it('lets the thread grow wider than the main column', () => {
    const viewport = 1280
    const nav = 300
    const thread = 600
    const room = maxThreadWidth(viewport, nav, 0)
    expect(room).toBe(viewport - RAIL_WIDTH - nav - MAIN_MIN)
    expect(clampWidth(thread, THREAD_MIN, room)).toBe(600)
    const main = viewport - RAIL_WIDTH - nav - thread
    expect(main).toBeGreaterThanOrEqual(MAIN_MIN)
    expect(main).toBeLessThan(thread)
  })

  it('lets the context nav grow past the old fixed cap when the main column can shrink', () => {
    const viewport = 1280
    const thread = 360
    const room = maxContextNav(viewport, thread, 0)
    expect(room).toBe(viewport - RAIL_WIDTH - MAIN_MIN - thread)
    expect(room).toBeGreaterThan(400)
    expect(clampWidth(700, NAV_MIN, room)).toBe(Math.min(700, room))
  })

  it('keeps a stacked thread at its saved width instead of reserving a side-by-side main column', () => {
    expect(paneRoom('thread', 900, 384, 0)).toBe(900)
    expect(clampWidth(540, THREAD_MIN, paneRoom('thread', 900, 384, 0))).toBe(540)
    expect(paneRoom('thread', 1280, 384, 0)).toBe(1280 - RAIL_WIDTH - 384 - MAIN_MIN)
  })

  it('shrinks below the preferred minimum when the window cannot fit every minimum', () => {
    expect(clampWidth(400, THREAD_MIN, 140)).toBe(140)
  })
})

describe('ticket panel width', () => {
  it('follows the thread: same smallest width and the same room rule', () => {
    expect(TICKET_MIN).toBe(THREAD_MIN)
    expect(maxTicketWidth(1440, 248)).toBe(maxThreadWidth(1440, 248, 0))
    expect(maxTicketWidth(1280, 300)).toBe(1280 - RAIL_WIDTH - 300 - MAIN_MIN)
    expect(maxTicketWidth(1920, 248)).toBeGreaterThan(1200)
    expect(clampWidth(40, TICKET_MIN, maxTicketWidth(1440, 248))).toBe(TICKET_MIN)
    expect(clampWidth(180, TICKET_MIN, maxTicketWidth(1440, 248))).toBe(180)
    expect(clampWidth(5000, TICKET_MIN, maxTicketWidth(1440, 248))).toBe(836)
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
