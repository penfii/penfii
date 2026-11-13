import { expect, test as base, type Page } from '@playwright/test'

type Watch = {
  /** Starts watching another page (one made with `browser.newContext()`); the test's own `page` is watched already. */
  watchPage: (page: Page) => void
  /** This test causes a problem on purpose: ignore the ones whose text matches. */
  allowProblem: (pattern: RegExp) => void
}

/**
 * `test` with a watch on the browser: a console error, an uncaught error in the page or a 5xx answer fails the
 * test, so a screen that works but logs a problem (or a request the server could not serve) does not go unseen.
 */
export const test = base.extend<Watch>({
  watchPage: [
    async ({ page }, provide) => {
      // WebKit does not know the `interactive-widget` viewport hint (it only tunes the on-screen keyboard elsewhere) and logs it.
      const allowed: RegExp[] = [/Viewport argument key "interactive-widget" not recognized/]
      const problems: string[] = []
      const note = (text: string) => {
        if (!allowed.some((pattern) => pattern.test(text))) problems.push(text)
      }
      const watch = (target: Page) => {
        target.on('console', (message) => {
          if (message.type() === 'error') note(`console.error: ${message.text()}`)
        })
        target.on('pageerror', (error) => note(`page error: ${error.message}`))
        target.on('response', (response) => {
          if (response.status() >= 500) note(`${response.status()} ${response.request().method()} ${response.url()}`)
        })
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
