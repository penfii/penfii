import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { runInNewContext } from 'node:vm'
import { describe, expect, it } from 'vitest'
import { deviceLabel, isIos, pushSupport, urlBase64ToBytes, type PushEnvironment } from './pushSupport'

// The worker is a plain script (it is served as /sw.js); it exports its logic only for this test.
const source = readFileSync(fileURLToPath(new URL('../../../../../apps/web/public/sw.js', import.meta.url)), 'utf8')
const sandbox = { module: { exports: {} as Record<string, unknown> }, URL }
runInNewContext(source, sandbox)
const sw = sandbox.module.exports as unknown as {
  readPush: (event: { data: { json: () => unknown } | null }) => Record<string, unknown>
  safePath: (path: unknown) => string
  isViewing: (windowUrl: string, targetPath: string) => boolean
  countsSilentPushes: (userAgent: unknown) => boolean
}

type Shown = { title: string; options: { tag: string | undefined; silent: boolean } }

/** The worker's push handler run against a stand-in for its global scope. */
async function pushed(
  userAgent: string,
  windows: { url: string; focused: boolean; visibilityState: string }[],
  { data = { v: 1, id: 'i1', title: 'Penfii', body: 'x', path: '/w/w1/c/c1' } as unknown, closeFails = false } = {},
) {
  const shown: Shown[] = []
  const closed: string[] = []
  const scope = {
    navigator: { userAgent },
    clients: { matchAll: async () => windows },
    registration: {
      showNotification: async (title: string, options: Shown['options']) => void shown.push({ title, options }),
      getNotifications: async ({ tag }: { tag: string }) => {
        if (closeFails) throw new Error('no notifications')
        return [{ close: () => void closed.push(tag) }]
      },
    },
  }
  const context = { module: { exports: {} as Record<string, unknown> }, URL, self: scope }
  runInNewContext(source, context)
  const show = context.module.exports.showUnlessViewing as (event: unknown) => Promise<void>
  await show({ data: { json: () => data } })
  return { shown, closed }
}

const env = (over: Partial<PushEnvironment> = {}): PushEnvironment => ({
  hasServiceWorker: true, hasPushManager: true, hasNotification: true,
  userAgent: 'Mozilla/5.0 (Windows NT 10.0) Chrome/120', platform: 'Win32', maxTouchPoints: 0, standalone: false, ...over,
})

describe('what this browser can do about Web Push', () => {
  it('supports a complete desktop browser and refuses one without Push', () => {
    expect(pushSupport(env())).toBe('supported')
    expect(pushSupport(env({ hasPushManager: false }))).toBe('unsupported')
    expect(pushSupport(env({ hasServiceWorker: false }))).toBe('unsupported')
  })

  it('asks iPhone and iPad (also an iPad that says it is a Mac) to add the app to the Home Screen first', () => {
    const iphone = { userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Safari', platform: 'iPhone' }
    const ipad = { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari', platform: 'MacIntel', maxTouchPoints: 5 }
    expect(isIos(env(iphone))).toBe(true)
    expect(isIos(env(ipad))).toBe(true)
    expect(isIos(env({ platform: 'MacIntel', maxTouchPoints: 0, userAgent: 'Macintosh' }))).toBe(false)
    expect(pushSupport(env({ ...iphone, hasPushManager: false }))).toBe('ios-needs-install')
    expect(pushSupport(env({ ...ipad, hasPushManager: false }))).toBe('ios-needs-install')
    // Installed: it can be switched on.
    expect(pushSupport(env({ ...iphone, standalone: true }))).toBe('supported')
  })

  it('turns the server key into bytes and names a device', () => {
    expect([...urlBase64ToBytes('AQID')]).toEqual([1, 2, 3])
    expect([...urlBase64ToBytes('-_8')]).toEqual([251, 255])
    expect(deviceLabel('Mozilla/5.0 (Windows NT 10.0) Chrome/120 Edg/120')).toBe('Edge · Windows')
    expect(deviceLabel('Mozilla/5.0 (iPhone) Safari/604')).toBe('Safari · iPhone')
  })
})

describe('the service worker', () => {
  it('handles push and notificationclick only: no fetch handler and no cache', () => {
    expect(source).toMatch(/addEventListener\('push'/)
    expect(source).toMatch(/addEventListener\('notificationclick'/)
    expect(source).not.toMatch(/addEventListener\(\s*['"]fetch['"]/)
    expect(source).not.toMatch(/caches\./)
    expect(source).not.toMatch(/skipWaiting|clients\.claim/)
  })

  it('reads a push message and falls back to a generic one', () => {
    expect(sw.readPush({ data: { json: () => ({ v: 1, id: 'i1', title: 'Penfii', body: 'x', path: '/w/a/inbox' }) } }).id).toBe('i1')
    const bad = sw.readPush({ data: { json: () => { throw new Error('not json') } } })
    expect(bad.title).toBe('Penfii')
    expect(sw.readPush({ data: null }).path).toBe('/')
  })

  it('opens only paths inside the app', () => {
    expect(sw.safePath('/w/a/inbox')).toBe('/w/a/inbox')
    expect(sw.safePath('https://evil.test/')).toBe('/')
    expect(sw.safePath('//evil.test/')).toBe('/')
    expect(sw.safePath(undefined)).toBe('/')
  })

  it('suppresses a notification only for the screen that handles it', () => {
    const O = 'https://app.test'
    // A thread: only that thread.
    expect(sw.isViewing(`${O}/w/w1/c/c1/t/m1`, '/w/w1/c/c1/t/m1')).toBe(true)
    expect(sw.isViewing(`${O}/w/w1/c/c1`, '/w/w1/c/c1/t/m1')).toBe(false)
    // A channel, also inside its Project, and with a thread open beside it? (the channel itself only)
    expect(sw.isViewing(`${O}/w/w1/c/c1`, '/w/w1/c/c1')).toBe(true)
    expect(sw.isViewing(`${O}/w/w1/projects/p1/chat/c1`, '/w/w1/c/c1')).toBe(true)
    expect(sw.isViewing(`${O}/w/w1/c/c2`, '/w/w1/c/c1')).toBe(false)
    // A Task: its panel.
    expect(sw.isViewing(`${O}/w/w1/projects/p1/work?panel=ticket:t1`, '/w/w1/projects/p1/work?panel=ticket:t1')).toBe(true)
    expect(sw.isViewing(`${O}/w/w1/projects/p1/work`, '/w/w1/projects/p1/work?panel=ticket:t1')).toBe(false)
    expect(sw.isViewing(`${O}/w/w1/projects/p1/work?panel=ticket:t2`, '/w/w1/projects/p1/work?panel=ticket:t1')).toBe(false)
    // A Doc; and the Inbox (the fallback) is not a screen that handles a Task.
    expect(sw.isViewing(`${O}/w/w1/docs?doc=d1`, '/w/w1/docs?doc=d1')).toBe(true)
    expect(sw.isViewing(`${O}/w/w1/docs?doc=d2`, '/w/w1/docs?doc=d1')).toBe(false)
    expect(sw.isViewing(`${O}/w/w1/inbox`, '/w/w1/projects/p1/work?panel=ticket:t1')).toBe(false)
  })

  const CHROMIUM = 'Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36 Edg/120'
  const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1'
  const here = [{ url: 'https://app.test/w/w1/c/c1', focused: true, visibilityState: 'visible' }]
  const elsewhere = [{ url: 'https://app.test/w/w1/inbox', focused: true, visibilityState: 'visible' }]

  it('knows which browsers end a subscription for pushes that show nothing', () => {
    expect(sw.countsSilentPushes(IPHONE)).toBe(true)
    expect(sw.countsSilentPushes('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/17.4 Safari/605.1.15')).toBe(true)
    expect(sw.countsSilentPushes(CHROMIUM)).toBe(false)
    expect(sw.countsSilentPushes('Mozilla/5.0 (iPhone) AppleWebKit/605 Version/17 EdgiOS/120 Mobile Safari/604')).toBe(false)
    expect(sw.countsSilentPushes(undefined)).toBe(false)
  })

  it('shows a push unless the open window already shows what it is about', async () => {
    expect((await pushed(CHROMIUM, elsewhere)).shown).toHaveLength(1)
    expect((await pushed(IPHONE, elsewhere)).shown[0].options.silent).toBe(false)
    // Chrome and Edge: nothing to show for the screen being looked at.
    expect((await pushed(CHROMIUM, here)).shown).toHaveLength(0)
  })

  it('on WebKit still shows the push for the screen being looked at, silently, and takes it down at once', async () => {
    const { shown, closed } = await pushed(IPHONE, here)
    expect(shown).toHaveLength(1)
    expect(shown[0].options.silent).toBe(true)
    expect(closed).toEqual(['inbox:i1'])
    // A push that is shown for real is not taken down.
    expect((await pushed(IPHONE, elsewhere)).closed).toEqual([])
  })

  it('does not fail the push event when the silent notification cannot be taken down, and keeps the tags', async () => {
    const failed = await pushed(IPHONE, here, { closeFails: true })
    expect(failed.shown).toHaveLength(1)
    // One notification per Inbox item; a push with no item gets none (but a tag when it must be found again).
    expect((await pushed(CHROMIUM, elsewhere)).shown[0].options.tag).toBe('inbox:i1')
    const unreadable = await pushed(CHROMIUM, elsewhere, { data: null })
    expect(unreadable.shown[0].options.tag).toBeUndefined()
  })
})
