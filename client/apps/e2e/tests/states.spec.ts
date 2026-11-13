import { expect, test } from '../support/test'
import { accountA } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// Screen states a person meets besides success: a failed load that can be retried, and a modal that keeps the keyboard.

test.describe('FEA-097: a failed load says so and can be retried', () => {
  test('the Docs list shows an error with Try again, and Try again loads it', async ({ page, request, allowProblem }) => {
    // The test makes the page list answer 500 twice.
    allowProblem(/status of 500/)
    allowProblem(/^500 GET .*\/docs\/tree/)
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await logIn(page, accountA.email)

    let failures = 2
    await page.route(new RegExp(`/v1/workspaces/${workspaceId}/docs/tree`), async (route) => {
      if (failures > 0 && route.request().method() === 'GET') {
        failures -= 1
        await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ code: 'internal', message: 'boom' }) })
        return
      }
      await route.continue()
    })
    await page.goto(`/w/${workspaceId}/docs`)
    const retry = page.getByRole('button', { name: 'Try again' }).first()
    await expect(retry).toBeVisible()
    // A failed list is not an empty one: no "create the first page" invitation next to the error.
    await expect(page.getByText('Create the first page')).toHaveCount(0)
    await retry.click()
    await expect(page.getByRole('tree', { name: 'Docs' })).toBeVisible({ timeout: 20_000 })
  })
})

test.describe('FEA-098: a modal takes the keyboard and gives it back', () => {
  test('the status dialog keeps Tab away from the page behind it, closes on Escape and returns focus to the account button', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}`)
    const account = page.getByRole('button', { name: accountA.name })
    await account.click()
    await page.getByRole('button', { name: 'Set a status' }).click()
    const dialog = page.getByRole('dialog')
    await expect(dialog).toBeVisible()

    // The page behind a modal dialog cannot take focus: after any number of Tab presses focus is inside the
    // dialog, or has left the page for the browser's own controls (then the document has no focused element).
    for (let press = 0; press < 12; press += 1) {
      await page.keyboard.press('Tab')
      const allowed = await page.evaluate(() => {
        const focused = document.activeElement
        return !focused || focused === document.body || Boolean(focused.closest('dialog'))
      })
      expect(allowed, `focus reached the page behind the dialog after ${press + 1} Tab presses`).toBe(true)
    }
    await page.keyboard.press('Escape')
    await expect(dialog).toHaveCount(0)
    await expect(page.getByRole('button', { name: accountA.name })).toBeFocused()
  })
})
