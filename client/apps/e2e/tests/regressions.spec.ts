import { request as playwrightRequest, type Page, type WebSocketRoute } from '@playwright/test'
import { expect, test } from '../support/test'
import { accountA, accountB, PASSWORD } from '../support/accounts'
import { apiAs, logIn, signedInContext } from '../support/api'

// The risks the frontend-quality audit found, each as the person would meet it. They are interaction and
// lifecycle problems, which component tests with a mocked API cannot show.

const column = (page: Page, name: string) => page.getByRole('region', { name, exact: true })

test.describe('FEA-088: a person never sees another person\'s data', () => {
  test('an expired session, then another person signing in on the same tab', async ({ browser, watchPage, allowProblem }) => {
    allowProblem(/status of 401/)
    const { context, page } = await signedInContext(browser, accountA.email, watchPage)
    await expect(page.getByRole('button', { name: accountA.name })).toBeVisible()
    // Something the first person left on this device that belongs to them (a draft, say).
    await page.evaluate(() => localStorage.setItem('penfii.e2e.canary', 'belongs to A'))

    // The session ends on the server (an expired cookie); the next request the page makes is refused.
    await context.clearCookies()
    await page.getByRole('navigation', { name: 'Workspace' }).getByRole('link', { name: 'Inbox' }).click()
    await expect(page).toHaveURL(/\/login/)

    await page.getByLabel('Email').fill(accountB.email)
    await page.getByLabel('Password').fill(PASSWORD)
    await page.getByRole('button', { name: 'Log in' }).click()
    await expect(page).not.toHaveURL(/\/login/)
    await expect(page.getByRole('button', { name: accountB.name })).toBeVisible()
    await expect(page.getByText(accountA.name)).toHaveCount(0)
    expect(await page.evaluate(() => localStorage.getItem('penfii.e2e.canary'))).toBeNull()
    await context.close()
  })

  test('logging out and signing in as someone else leaves nothing of the first person', async ({ browser, watchPage }) => {
    const { context, page } = await signedInContext(browser, accountA.email, watchPage)
    await page.evaluate(() => localStorage.setItem('penfii.e2e.canary', 'belongs to A'))
    await page.getByRole('button', { name: accountA.name }).click()
    await page.getByRole('button', { name: 'Sign out' }).click()
    await expect(page).toHaveURL(/\/login/)
    expect(await page.evaluate(() => localStorage.getItem('penfii.e2e.canary'))).toBeNull()
    await context.close()
  })
})

test.describe('FEA-089: crossing 640px keeps what is on screen', () => {
  test('unsaved edits in a document survive a switch to the phone layout and back', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const doc = await api.createDoc(workspaceId, `E2E resize ${Date.now()}`)
    const text = `typed before resizing ${Date.now()}`
    await page.setViewportSize({ width: 1280, height: 800 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
    await page.getByRole('button', { name: 'Edit' }).click()
    const editor = page.locator('.ProseMirror[contenteditable="true"]').first()
    await editor.click()
    await page.keyboard.type(text)
    await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toBeVisible()

    await page.setViewportSize({ width: 600, height: 900 })
    await expect(page.locator('.ProseMirror[contenteditable="true"]').first()).toContainText(text)
    await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toBeVisible()
    await page.setViewportSize({ width: 1280, height: 800 })
    await expect(page.locator('.ProseMirror[contenteditable="true"]').first()).toContainText(text)
    await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toBeVisible()
  })

  test('an open ticket and a comment being written stay put across the boundary', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E resize ticket ${Date.now()}`)
    const ticket = await api.createTicket(project.id, 'Resize me')
    const text = `half a comment ${Date.now()}`
    await page.setViewportSize({ width: 1280, height: 800 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    const comment = page.getByRole('textbox', { name: /comment/i }).first()
    await comment.click()
    await page.keyboard.type(text)

    await page.setViewportSize({ width: 600, height: 900 })
    await expect(page.getByRole('textbox', { name: /comment/i }).first()).toContainText(text)
    await expect(page).toHaveURL(new RegExp(`panel=ticket:${ticket.id}`))
    await page.setViewportSize({ width: 1280, height: 800 })
    await expect(page.getByRole('textbox', { name: /comment/i }).first()).toContainText(text)
  })
})

test.describe('FEA-090 / FEA-091: the board follows other people and catches up after a gap', () => {
  test('a ticket moved elsewhere moves here, also after the network was away', async ({ browser, request, watchPage }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E realtime ${Date.now()}`)
    const ticket = await api.createTicket(project.id, 'Moves around')
    const board = await api.board(project.id)
    const state = (name: string) => board.columns.find((item) => item.name === name)!.stateId

    const { context, page } = await signedInContext(browser, accountA.email, watchPage)
    // Every live socket passes through here, so the test can cut them and keep new ones from connecting (a real gap;
    // `setOffline` does not close a socket that is already open).
    let reachable = true
    const sockets: WebSocketRoute[] = []
    await page.routeWebSocket(/\/v1\/ws/, (route) => {
      if (!reachable) {
        void route.close()
        return
      }
      route.connectToServer()
      sockets.push(route)
    })
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work`)
    await expect(column(page, 'Todo').getByText('Moves around')).toBeVisible()
    await expect.poll(() => sockets.length, { message: 'the board opened its live socket' }).toBeGreaterThan(0)

    // Another session moves it (a second browser, here the API with its own login; the Project is A's, so it is A's session).
    const other = await (async () => {
      const second = await playwrightRequest.newContext({ baseURL: process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080' })
      return { api: await apiAs(second, accountA.email), dispose: () => second.dispose() }
    })()
    let revision = ticket.revisionNo
    const moved = (await other.api.moveTicket(ticket.id, state('In Progress'), revision)) as { revisionNo: number }
    revision = moved.revisionNo
    await expect(column(page, 'In Progress').getByText('Moves around')).toBeVisible()
    await expect(column(page, 'Todo').getByText('Moves around')).toHaveCount(0)

    // The network goes away; the change happens meanwhile; when it is back, the board catches up without a reload.
    reachable = false
    for (const route of sockets.splice(0)) void route.close()
    await other.api.moveTicket(ticket.id, state('Done'), revision)
    // Nothing reached this page while the socket was away.
    await page.waitForTimeout(1500)
    await expect(column(page, 'In Progress').getByText('Moves around')).toBeVisible()
    reachable = true
    await expect(column(page, 'Done').getByText('Moves around')).toBeVisible({ timeout: 45_000 })
    await expect(column(page, 'In Progress').getByText('Moves around')).toHaveCount(0)
    await other.dispose()
    await context.close()
  })
})

test.describe('FEA-093: moving a card on the board', () => {
  test('the card moves at once with the keyboard and the server agrees', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E move ${Date.now()}`)
    const ticket = await api.createTicket(project.id, 'Keyboard move')
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work`)
    const card = column(page, 'Todo').getByRole('button', { name: new RegExp(`Open ${ticket.key}`) })
    await card.focus()
    await page.keyboard.press('Alt+ArrowRight')
    await expect(column(page, 'In Progress').getByText('Keyboard move')).toBeVisible()
    await expect(column(page, 'Todo').getByText('Keyboard move')).toHaveCount(0)
    await expect.poll(async () => (await api.board(project.id)).columns.find((item) => item.name === 'In Progress')?.tickets.length).toBe(1)
  })
})

test.describe('FEA-093: a move the server refuses', () => {
  test('the card goes back to its column and the person is told, and nothing was changed on the server', async ({ page, request, allowProblem }) => {
    allowProblem(/status of 500/)
    allowProblem(/^500 POST .*\/commands\/move/)
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E rollback ${Date.now()}`)
    const ticket = await api.createTicket(project.id, 'Refused move')
    await logIn(page, accountA.email)
    // The server fails the move after a moment, so the card is seen in the new column first (the optimistic update).
    await page.route(/\/v1\/tickets\/[^/]+\/commands\/move/, async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 600))
      await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ code: 'internal', message: 'boom' }) })
    })
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work`)
    const card = column(page, 'Todo').getByRole('button', { name: new RegExp(`Open ${ticket.key}`) })
    await card.focus()
    await page.keyboard.press('Alt+ArrowRight')
    await expect(column(page, 'In Progress').getByText('Refused move')).toBeVisible()
    await expect(column(page, 'Todo').getByText('Refused move')).toBeVisible({ timeout: 15_000 })
    await expect(column(page, 'In Progress').getByText('Refused move')).toHaveCount(0)
    await expect(page.getByRole('alert').first()).toBeVisible()
    const board = await api.board(project.id)
    expect(board.columns.find((item) => item.name === 'Todo')?.tickets.map((item) => item.id)).toContain(ticket.id)
  })
})

test.describe('FEA-094: a draft does not come back after the message was sent', () => {
  test('a slow save of the draft that lands after sending changes nothing', async ({ page, request, allowProblem }) => {
    // Depending on which request reaches the server first, the late save is refused with 409 draft_closed; a browser logs that.
    allowProblem(/status of 409/)
    allowProblem(/^409 PUT .*\/v1\/drafts/)
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `e2e-draft-${Date.now()}`)
    const text = `slow draft ${Date.now()}`

    await logIn(page, accountA.email)
    // The first save of this text is held back by the "network"; the message is sent while it is in flight.
    let release: () => void = () => undefined
    const gate = new Promise<void>((resolve) => { release = resolve })
    let heldSave = false
    await page.route('**/v1/drafts', async (route) => {
      if (route.request().method() === 'PUT' && !heldSave) {
        heldSave = true
        await gate
      }
      await route.continue()
    })
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    const box = page.getByRole('textbox', { name: /message/i }).first()
    await box.click()
    await page.keyboard.type(text)
    await expect.poll(() => heldSave, { timeout: 15_000 }).toBe(true)
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(page.getByRole('article').getByText(text)).toBeVisible()
    const answered = (method: string) => page.waitForResponse((response) => response.url().includes('/v1/drafts') && response.request().method() === method)
    const [saved, closed] = [answered('PUT'), answered('DELETE')]
    release()
    // Both have been answered (in whichever order) before the page is left; the session is over on the server by then.
    await Promise.all([saved, closed])

    // After the slow save has landed: nothing is left of the draft, on this screen, after a reload, or on the server.
    await page.reload()
    await expect(page.getByRole('textbox', { name: /message/i }).first()).toHaveText('')
    await expect(page.getByRole('article').getByText(text)).toBeVisible()
    const stored = await api.draft('CHANNEL_COMPOSE', channel.conversationId)
    expect(JSON.stringify(stored?.payload ?? '')).not.toContain(text)
  })

  test('a draft follows its conversation across a switch and is gone once the box is emptied', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const first = await api.createChannel(workspaceId, `e2e-ctx-a-${Date.now()}`)
    const second = await api.createChannel(workspaceId, `e2e-ctx-b-${Date.now()}`)
    const text = `unfinished thought ${Date.now()}`
    const box = () => page.getByRole('textbox', { name: /message/i }).first()

    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${first.conversationId}`)
    await box().click()
    await page.keyboard.type(text)
    // Switching away flushes the draft of the conversation being left; the other one starts empty.
    // The side list shows the newest channels first; open the full list if this one is behind "Show more".
    const target = page.getByRole('link', { name: `# ${second.name}` })
    if ((await target.count()) === 0) await page.getByRole('button', { name: 'Show more' }).first().click()
    await target.click()
    await expect(page).toHaveURL(new RegExp(second.conversationId))
    await expect(box()).toHaveText('')
    await expect.poll(async () => JSON.stringify((await api.draft('CHANNEL_COMPOSE', first.conversationId))?.payload ?? '')).toContain(text)
    expect(await api.draft('CHANNEL_COMPOSE', second.conversationId)).toBeNull()

    // Back again: the text is in the box. Emptying the box ends that draft.
    await page.goto(`/w/${workspaceId}/c/${first.conversationId}`)
    await expect(box()).toContainText(text)
    await box().click()
    await page.keyboard.press('Control+A')
    await page.keyboard.press('Delete')
    await expect.poll(async () => JSON.stringify((await api.draft('CHANNEL_COMPOSE', first.conversationId))?.payload ?? '')).not.toContain(text)
    await page.reload()
    await expect(box()).toHaveText('')
  })

  test('sending and reloading at once leaves no draft behind', async ({ page, request, allowProblem }) => {
    // The reload cancels what is in flight on the page; some engines log that.
    allowProblem(/access control checks|cancelled|aborted|NS_BINDING_ABORTED/i)
    allowProblem(/status of 409/)
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `e2e-reload-${Date.now()}`)
    const text = `sent then reloaded ${Date.now()}`
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    const box = page.getByRole('textbox', { name: /message/i }).first()
    await box.click()
    await page.keyboard.type(text)
    // The draft is on the server (the save has landed), then the message goes out and the page is reloaded without waiting.
    await expect.poll(async () => JSON.stringify((await api.draft('CHANNEL_COMPOSE', channel.conversationId))?.payload ?? '')).toContain(text)
    // Once the server has taken the message the box is emptied and the session closed; the page is left that same moment,
    // so the close request is still in flight (it must be finished by the browser, not dropped with the page).
    const posted = page.waitForResponse((response) => response.request().method() === 'POST' && /\/messages$/.test(new URL(response.url()).pathname))
    await page.keyboard.press('Enter')
    await posted
    await page.reload()
    await expect.poll(async () => JSON.stringify((await api.draft('CHANNEL_COMPOSE', channel.conversationId))?.payload ?? ''), { timeout: 10_000 }).not.toContain(text)
    await expect(page.getByRole('textbox', { name: /message/i }).first()).toHaveText('')
  })
})

