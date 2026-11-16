import { expect, test as base, type Page } from '@playwright/test'

type Watch = {
  /** Starts watching another page (one made with `browser.newContext()`); the test's own `page` is watched already. */
  watchPage: (page: Page) => void
  /** This test causes a problem on purpose: ignore the ones whose text matches. */
  allowProblem: (pattern: RegExp) => void
}

/**
 * `test` with a watch on the browser: console errors/warnings, uncaught errors, failed requests and HTTP errors fail
 * the test, so a screen that looks right cannot hide a browser or server problem.
 */
export const test = base.extend<Watch>({
  watchPage: [
    async ({ page }, provide) => {
      // WebKit does not know the `interactive-widget` viewport hint (it only tunes the on-screen keyboard elsewhere).
      const allowed: RegExp[] = [
        /Viewport argument key "interactive-widget" not recognized/,
        /Service Worker registration blocked by Playwright/,
        /^navigation cancellation:/,
        /^navigation WebSocket close:/,
        /scroll-linked positioning effect/,
      ]
      const problems: string[] = []
      let lastMainNavigationAt = 0
      const note = (text: string) => {
        if (!allowed.some((pattern) => pattern.test(text))) {
          problems.push(text)
          console.log(`[browser observation] ${text}`)
        } else {
          console.log(`[browser warning reviewed] ${text}`)
        }
      }
      const watch = (target: Page) => {
        // A navigation (goto, reload, a link) ends the old document's requests and sockets before the new one
        // commits, so its start counts as well as its commit.
        target.on('request', (request) => {
          if (request.isNavigationRequest() && request.frame() === target.mainFrame()) lastMainNavigationAt = Date.now()
        })
        target.on('framenavigated', (frame) => {
          if (frame === target.mainFrame()) lastMainNavigationAt = Date.now()
        })
        target.on('console', (message) => {
          if (message.type() === 'error' || message.type() === 'warning') note(`console.${message.type()}: ${message.text()}`)
        })
        target.on('pageerror', (error) => note(`page error: ${error.message}`))
        target.on('requestfailed', (request) => {
          const reason = request.failure()?.errorText ?? 'unknown'
          const recentNavigation = Date.now() - lastMainNavigationAt < 2_000
          if (recentNavigation && /Load request cancelled|NS_BINDING_ABORTED|ERR_ABORTED/.test(reason)) {
            note(`navigation cancellation: ${request.method()} ${request.url()} (${reason})`)
          } else {
            note(`request failed: ${request.method()} ${request.url()} (${reason})`)
          }
        })
        target.on('response', (response) => {
          if (response.status() >= 400) note(`${response.status()} ${response.request().method()} ${response.url()}`)
        })
        target.on('websocket', (socket) => socket.on('socketerror', (error) => {
          const recentNavigation = Date.now() - lastMainNavigationAt < 2_000
          if (recentNavigation && String(error).includes('CLOSE_ABNORMAL')) {
            note(`navigation WebSocket close: ${socket.url()} (${error}) at ${target.url()}`)
          } else {
            note(`WebSocket error: ${socket.url()} (${error}) at ${target.url()}`)
          }
        }))
      }
      watch(page)
      allowedPatterns.set(page, allowed)
      await provide(watch)
      expect(problems, 'the browser reported problems').toEqual([])
    },
    { auto: true },
  ],
  allowProblem: async ({ page, watchPage: _watch }, provide) => {
    await provide((pattern) => { allowedPatterns.get(page)?.push(pattern) })
  },
})

const allowedPatterns = new WeakMap<Page, RegExp[]>()

export { expect }
