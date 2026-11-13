import { expect, test } from '../support/test'
import { accountA } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// What a browser reports while an input method is composing, sent as the events a real one would send: Enter with
// `isComposing`, and Safari's late Enter after the composition ended (`keyCode` 229). This checks how the page treats
// those events in each engine. It is NOT a real Korean/Japanese/Chinese IME: that cannot be driven from here.
const composingEnter = (isComposing: boolean, keyCode: number) =>
  `new KeyboardEvent('keydown', { key: 'Enter', isComposing: ${isComposing}, keyCode: ${keyCode}, bubbles: true, cancelable: true })`

test.describe('Enter that belongs to an input method (FEA-008)', () => {
  test('does not jump to a result from the command palette, but a plain Enter does', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}`)
    await expect(page.getByRole('navigation', { name: 'Workspace' })).toBeVisible()
    await page.keyboard.press('Control+k')
    const box = page.getByRole('combobox')
    await expect(box).toBeVisible()
    await box.fill('inbox')
    for (const [composing, code] of [[true, 229], [false, 229]] as const) {
      await box.evaluate((element, source) => element.dispatchEvent(eval(source) as Event), composingEnter(composing, code))
      await expect(page).toHaveURL(new RegExp(`/w/${workspaceId}$`))
    }
    await box.press('Enter')
    await expect(page).toHaveURL(/\/inbox$/)
  })

  test('does not send the message while the input method is still composing, but a plain Enter does', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `e2e-ime-${Date.now()}`)
    const text = `composed ${Date.now()}`
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    const box = page.getByRole('textbox', { name: /message/i }).first()
    await box.click()
    await page.keyboard.type(text)
    await box.evaluate((element, source) => element.dispatchEvent(eval(source) as Event), composingEnter(true, 229))
    await page.waitForTimeout(500)
    expect(JSON.stringify(await api.messages(channel.conversationId))).not.toContain(text)
    await page.keyboard.press('Enter')
    await expect(page.getByRole('article').getByText(text)).toBeVisible()
  })
})
