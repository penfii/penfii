import { expect, test } from '../support/test'
import { accountA } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// Core flows through the real screens: log in -> board -> edit a ticket, send a chat message, write and save a doc.
test.describe('core flows', () => {
  test('log in, see the board, open a ticket and rename it', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E board ${Date.now()}`)
    const ticket = await api.createTicket(project.id, 'First ticket')

    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work`)
    await page.getByRole('button', { name: new RegExp(ticket.key) }).first().click()
    const title = page.getByRole('textbox', { name: /title/i }).first()
    await expect(title).toHaveValue('First ticket')
    await title.fill('Renamed by the browser')
    await title.press('Enter')
    await expect(page.getByText('Renamed by the browser').first()).toBeVisible()

    await page.reload()
    await expect(page.getByText('Renamed by the browser').first()).toBeVisible()
    const board = await api.board(project.id)
    expect(board.columns.flatMap((column) => column.tickets).map((item) => item.title)).toContain('Renamed by the browser')
  })

  test('send a chat message and see it in the conversation', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `e2e-${Date.now()}`)
    const text = `hello from the browser ${Date.now()}`

    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    const box = page.getByRole('textbox', { name: /message/i }).first()
    await box.click()
    await page.keyboard.type(text)
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(page.getByRole('article').getByText(text)).toBeVisible()
    // The box is empty again once the message is sent, and stays empty after a reload (the draft was closed, not kept).
    await expect(box).toHaveText('')
    await page.reload()
    await expect(page.getByRole('article').getByText(text)).toBeVisible()
    await expect(page.getByRole('textbox', { name: /message/i }).first()).toHaveText('')

    const list = JSON.stringify(await api.messages(channel.conversationId))
    expect(list).toContain(text)
  })

  test('open a document, edit it and save', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const doc = await api.createDoc(workspaceId, `E2E doc ${Date.now()}`)
    const text = `written in the browser ${Date.now()}`

    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
    await page.getByRole('button', { name: 'Edit' }).click()
    const editor = page.locator('.ProseMirror[contenteditable="true"]').first()
    await editor.click()
    await page.keyboard.type(text)
    await page.getByRole('button', { name: 'Save' }).click()
    await expect(page.getByRole('status').filter({ hasText: 'Saved' })).toBeVisible()

    await page.reload()
    await expect(page.getByText(text).first()).toBeVisible()
    expect(JSON.stringify((await api.doc(doc.id)).content)).toContain(text)
  })
})
