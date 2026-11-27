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
        // The sign-in page asks the server whether a session exists (to take a signed-in person on); for a person who is
        // signed out the answer is 401, and the browser reports that as a failed request and a console error.
        /^sign-in page session check:/,
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
        // Only `/v1/me`, only answered 401, only while the sign-in page is open: any other failure there still counts.
        const isSignInSessionCheck = (url: string) => new URL(url).pathname === '/v1/me' && new URL(target.url()).pathname === '/login'
        target.on('console', (message) => {
          if (message.type() !== 'error' && message.type() !== 'warning') return
          const where = message.location().url
          const sessionCheck = message.type() === 'error' && /status of 401/.test(message.text()) && Boolean(where) && isSignInSessionCheck(where)
          // WebKit also logs what a navigation cut off: a lazily loaded screen whose script request was cancelled, and a socket
          // still connecting; Firefox logs that socket as well, in its own words. The request itself is checked above (a
          // cancellation only right after a navigation; a real failure or an HTTP error still counts), so only these messages,
          // only right after a navigation, are that.
          const recentNavigation = Date.now() - lastMainNavigationAt < 2_000
          const cutOff = recentNavigation && (
            /^(TypeError: Importing a module script failed\.|WebSocket connection to .* failed: WebSocket is closed before the connection is established\.)$/.test(message.text())
            || /^\[JavaScript Error: "(The connection to ws:\/\/\S+ was interrupted while the page was loading\.|Firefox can.t establish a connection to the server at ws:\/\/\S+\.)"/.test(message.text())
          )
          note(`${sessionCheck ? 'sign-in page session check: ' : cutOff ? 'navigation cancellation: ' : ''}console.${message.type()}: ${message.text()}`)
        })
        target.on('pageerror', (error) => {
          // WebKit reports a fetch that a navigation cut off as "... due to access control checks"; the app's query
          // handles the rejection. Only right after a navigation does it count as that.
          const recentNavigation = Date.now() - lastMainNavigationAt < 2_000
          note(recentNavigation && /due to access control checks/.test(error.message) ? `navigation cancellation: page error: ${error.message}` : `page error: ${error.message}`)
        })
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
          if (response.status() < 400) return
          const sessionCheck = response.status() === 401 && isSignInSessionCheck(response.url())
          note(`${sessionCheck ? 'sign-in page session check: ' : ''}${response.status()} ${response.request().method()} ${response.url()}`)
        })
        target.on('websocket', (socket) => socket.on('socketerror', (error) => {
          const recentNavigation = Date.now() - lastMainNavigationAt < 2_000
          if (recentNavigation && /CLOSE_ABNORMAL|closed before the connection is established/.test(String(error))) {
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
