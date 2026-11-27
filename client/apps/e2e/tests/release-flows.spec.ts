import { randomUUID } from 'node:crypto'
import { request as playwrightRequest, type APIRequestContext, type Browser, type Page } from '@playwright/test'
import { expect, test } from '../support/test'
import { accountA, PASSWORD } from '../support/accounts'
import { apiAs, logIn, type Api } from '../support/api'
import { projectSocketReady } from '../support/multiUser'

// Flows the other specs leave out, written for the v0.2.1 release check: invitations and password reset by e-mail,
// a Ticket's archive, trash, restore and purge, recovering a private channel nobody manages, the Web Push switch, the
// phone layout and a menu whose trigger leaves the viewport. They run on every engine of E2E_BROWSERS.
//
// The e-mail flows need an SMTP catcher the stack sends to and whose HTTP API the test reads (Mailpit):
// E2E_MAILPIT=http://127.0.0.1:8025 with PENFII_SMTP_HOST pointing at it. Without it they are skipped.

const mailpit = process.env.E2E_MAILPIT
const HEADERS = { 'x-penfii-client': '1' }
const stamp = () => `${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`

async function send(request: APIRequestContext, method: string, path: string, body?: unknown) {
  const response = await request.fetch(path, { method, headers: { ...HEADERS, 'idempotency-key': randomUUID() }, data: body })
  const text = await response.text()
  expect(response.status(), `${method} ${path}: ${text}`).toBeLessThan(300)
  return (text ? JSON.parse(text) : undefined) as Record<string, unknown>
}

/** The newest mail to `address` that has a link to `path`, once it has arrived. */
async function linkMailedTo(address: string, path: RegExp): Promise<string> {
  let link = ''
  await expect.poll(async () => {
    const search = await fetch(`${mailpit}/api/v1/search?query=${encodeURIComponent(`to:"${address}"`)}`)
    const found = (await search.json()) as { messages: Array<{ ID: string }> }
    for (const message of found.messages) {
      const full = (await (await fetch(`${mailpit}/api/v1/message/${message.ID}`)).json()) as { Text: string }
      const match = full.Text.match(new RegExp(`https?://\\S+${path.source}\\S*`))
      if (match) {
        link = match[0]
        return true
      }
    }
    return false
  }, { message: `a mail to ${address} with a ${path} link`, timeout: 15_000 }).toBe(true)
  return link
}

/** A fresh context that is nobody yet (no cookies), watched like the test's own page. */
async function stranger(browser: Browser, watch: (page: Page) => void) {
  const context = await browser.newContext({ locale: 'en-US', serviceWorkers: 'block' })
  const page = await context.newPage()
  watch(page)
  return { context, page }
}

const inViewport = (page: Page, selector: string) =>
  page.locator(selector).first().evaluate((element) => {
    const rect = element.getBoundingClientRect()
    return rect.top >= 0 && rect.left >= 0 && rect.bottom <= window.innerHeight + 0.5 && rect.right <= window.innerWidth + 0.5
  })

const noSidewaysScroll = (page: Page) => page.evaluate(() => document.scrollingElement!.scrollWidth <= window.innerWidth)

test.describe('invitation, password reset and recovery (e-mail)', () => {
  test.skip(!mailpit, 'set E2E_MAILPIT to the Mailpit the stack sends to')
  test.describe.configure({ mode: 'serial' })

  let workspaceId = ''
  let invitee = { email: '', name: '', password: '' }

  test('an owner invites by e-mail and the invited person joins through the mailed link', async ({ page, request, browser, browserName, watchPage, allowProblem }) => {
    const id = stamp()
    invitee = { email: `inv-${browserName}-${id}@example.test`, name: `Invitee ${browserName} ${id}`, password: `Invitee-pass-${id}-long` }
    workspaceId = await (await apiAs(request, accountA.email)).workspaceId()
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/settings/admin/invitations`)
    await page.getByRole('button', { name: 'Invite member' }).click()
    const dialog = page.getByRole('dialog', { name: 'Invite member' })
    await dialog.getByRole('textbox', { name: 'Email' }).fill(invitee.email)
    await dialog.getByRole('button', { name: 'Invite member' }).click()
    await expect(dialog.getByRole('status')).toContainText(`An invitation e-mail was sent to ${invitee.email}.`)
    const shown = await dialog.getByRole('textbox', { name: 'Invitation link' }).inputValue()
    await dialog.getByRole('button', { name: 'Close' }).click()
    await expect(page.getByText(invitee.email)).toBeVisible()

    const mailed = await linkMailedTo(invitee.email, /\/invite\//)
    expect(mailed, 'the mail carries the same link the inviter saw').toBe(shown)

    // Somebody signed in to nothing yet: the app's look at the session is refused.
    allowProblem(/401 GET .*\/v1\/me/)
    allowProblem(/Failed to load resource: the server responded with a status of 401/)
    const { context, page: guest } = await stranger(browser, watchPage)
    await guest.goto(new URL(mailed).pathname)
    await expect(guest.getByText(`Invitation to`)).toContainText(invitee.email)
    await guest.getByLabel('Display name').fill(invitee.name)
    await guest.getByLabel('Password').fill(invitee.password)
    await guest.getByRole('button', { name: 'Join' }).click()
    await expect(guest.getByText('You joined.')).toBeVisible()
    // Accepting makes the account but no session (as in v0.1.0): Continue leads to the login screen.
    await guest.getByRole('button', { name: 'Continue' }).click()
    await expect(guest).toHaveURL(/\/login/)
    await guest.getByLabel('Email').fill(invitee.email)
    await guest.getByLabel('Password').fill(invitee.password)
    await guest.getByRole('button', { name: 'Log in' }).click()
    await expect(guest).toHaveURL(new RegExp(`/w/${workspaceId}`))
    await expect(guest.getByRole('button', { name: invitee.name, exact: true })).toBeVisible()

    // The link works once. A used link says only "Not found" (as in v0.1.0; known issue, no reason is given).
    allowProblem(/404 POST .*\/v1\/invitations\/preview/)
    allowProblem(/Failed to load resource: the server responded with a status of 404/)
    await guest.goto(new URL(mailed).pathname)
    await expect(guest.locator('[data-state="not-found"]')).toBeVisible()
    await context.close()
  })

  test('the invited person resets a forgotten password through the mailed link, once', async ({ browser, watchPage, allowProblem }) => {
    expect(invitee.email, 'the invitation test ran first').not.toBe('')
    allowProblem(/401 (GET .*\/v1\/me|POST .*\/v1\/session\/login)/)
    allowProblem(/Failed to load resource: the server responded with a status of 401/)
    const { context, page } = await stranger(browser, watchPage)
    await page.goto('/login')
    await page.getByRole('link', { name: 'Forgot your password?' }).click()
    await expect(page.getByRole('heading', { name: 'Reset your password' })).toBeVisible()
    await page.getByLabel('Email').fill(invitee.email)
    await page.getByRole('button', { name: 'Send reset link' }).click()
    await expect(page.getByRole('status').filter({ hasText: 'If there is an account' })).toContainText('If there is an account for that address, a reset link is on its way.')

    const link = await linkMailedTo(invitee.email, /\/reset-password/)
    const changed = `Changed-pass-${stamp()}-long`
    await page.goto(new URL(link).pathname + new URL(link).search + new URL(link).hash)
    await expect(page.getByRole('heading', { name: 'Choose a new password' })).toBeVisible()
    await page.getByLabel('New password', { exact: true }).fill(changed)
    await page.getByLabel('New password again').fill(`${changed}-typo`)
    await page.getByRole('button', { name: 'Set password' }).click()
    await expect(page.getByText('The two passwords are not the same.')).toBeVisible()
    await page.getByLabel('New password again').fill(changed)
    await page.getByRole('button', { name: 'Set password' }).click()
    await expect(page.getByRole('status').filter({ hasText: 'Your password was changed.' })).toBeVisible()

    // The old password is refused, the new one works.
    await page.goto('/login')
    await page.getByLabel('Email').fill(invitee.email)
    await page.getByLabel('Password').fill(invitee.password)
    await page.getByRole('button', { name: 'Log in' }).click()
    await expect(page.getByText('The email or password is incorrect')).toBeVisible()
    await page.getByLabel('Password').fill(changed)
    await page.getByRole('button', { name: 'Log in' }).click()
    await expect(page.getByRole('button', { name: invitee.name, exact: true })).toBeVisible()
    invitee.password = changed

    // The same link a second time.
    allowProblem(/409 POST .*\/v1\/auth\/password-reset\/confirm/)
    allowProblem(/Failed to load resource: the server responded with a status of 409/)
    const again = await stranger(browser, watchPage)
    await again.page.goto(new URL(link).pathname + new URL(link).search + new URL(link).hash)
    await again.page.getByLabel('New password', { exact: true }).fill(`${changed}-2`)
    await again.page.getByLabel('New password again').fill(`${changed}-2`)
    await again.page.getByRole('button', { name: 'Set password' }).click()
    await expect(again.page.getByRole('alert')).toContainText('This reset link no longer works.')
    await again.context.close()
    await context.close()
  })

  test('a private channel whose only manager was disabled is listed under Recovery and gets a manager again', async ({ page, request }) => {
    expect(invitee.email, 'the invitation test ran first').not.toBe('')
    const owner = await apiAs(request, accountA.email)
    const ownerId = (await owner.me()).id
    const inviteeRequest = await playwrightRequest.newContext({ baseURL: test.info().project.use.baseURL })
    await send(inviteeRequest, 'POST', '/v1/session/login', { email: invitee.email, password: invitee.password })
    const inviteeId = (await send(inviteeRequest, 'GET', '/v1/me')).id as string
    await inviteeRequest.dispose()
    const channel = await owner.createChannel(workspaceId, `rec-${stamp()}`, 'PRIVATE')
    await owner.addChannelMember(channel.id, inviteeId)
    await send(request, 'PATCH', `/v1/channels/${channel.id}/members/${inviteeId}`, { role: 'MANAGER' })
    await send(request, 'DELETE', `/v1/channels/${channel.id}/members/${ownerId}`)

    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/settings/admin/members`)
    await page.getByRole('searchbox', { name: 'Search by name, e-mail or team' }).fill(invitee.email)
    await page.getByRole('button', { name: `Actions for ${invitee.name} (${invitee.email})` }).click()
    await page.getByRole('menuitem', { name: 'Disable account' }).click()
    const confirm = page.getByRole('alertdialog').or(page.getByRole('dialog').filter({ hasText: 'Disable the account' })).last()
    await confirm.getByRole('button', { name: /Disable/ }).click()
    await expect(page.getByText('Account disabled')).toBeVisible()

    await page.goto(`/w/${workspaceId}/settings/admin/recovery`)
    const row = page.locator('li.admin-no-admin-row').filter({ hasText: channel.name })
    await expect(row).toContainText(`Former managers: ${invitee.name}`)
    await row.getByRole('combobox', { name: `Person to name manager: ${channel.name}` }).selectOption({ label: `${accountA.name} (${accountA.email})` })
    await row.getByRole('button', { name: `Name manager: ${channel.name}` }).click()
    const ask = page.getByRole('alertdialog').or(page.getByRole('dialog').filter({ hasText: 'Name yourself manager' })).last()
    await expect(ask).toContainText(`Name yourself manager of the private channel "${channel.name}"?`)
    await ask.getByRole('button', { name: 'Name manager' }).click()
    await expect(row).toHaveCount(0)

    // Named manager, the owner reads and runs the channel again.
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    await expect(page.getByRole('heading', { name: `# ${channel.name}`, level: 1 })).toBeVisible()
    await expect(page.getByRole('textbox', { name: 'Write a message' })).toBeVisible()
  })
})

test.describe('a Ticket kept, binned and brought back', () => {
  test('archive, open the archive, unarchive, move to trash, restore from the trash, then purge', async ({ page, request }) => {
    const api: Api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `Lifecycle ${stamp()}`)
    const ticket = await api.createTicket(project.id, `Kept and binned ${stamp()}`)
    await logIn(page, accountA.email)
    const work = `/w/${workspaceId}/projects/${project.id}/work`
    // The Project socket's first `project.ready` reads the open Ticket again. A trash sent while that read is on its way makes
    // the read answer 404 (the Ticket is gone by then, as for a trash by somebody else), so each visit waits for it first.
    const openPanel = async () => {
      const ready = projectSocketReady(page)
      await page.goto(`${work}?panel=ticket:${ticket.id}`)
      await ready
    }
    await openPanel()
    await expect(page.getByRole('textbox', { name: 'Title' })).toHaveValue(/Kept and binned/)

    await page.getByRole('button', { name: 'Archive', exact: true }).click()
    const archiveAsk = page.getByRole('alertdialog').or(page.getByRole('dialog').filter({ hasText: 'Archive ticket' })).last()
    await expect(archiveAsk).toContainText(`Archive ${ticket.key}?`)
    await archiveAsk.getByRole('button', { name: 'Archive' }).click()
    await expect(page.getByText('This ticket is archived. It is read-only until it is unarchived.')).toBeVisible()
    // Read-only: the title is no longer a field and the description cannot be edited.
    await expect(page.getByRole('textbox', { name: 'Title' })).toHaveCount(0)
    await expect(page.getByRole('button', { name: 'Edit description' })).toHaveCount(0)
    await expect(page.getByRole('button', { name: `Open ${ticket.key}` })).toHaveCount(0)

    await page.getByRole('button', { name: 'Archived tickets' }).click()
    await expect(page.getByRole('heading', { name: 'Archived tickets' })).toBeVisible()
    await expect(page.getByRole('link', { name: new RegExp(ticket.key) }).or(page.getByText(ticket.key)).first()).toBeVisible()

    // The live update from Unarchive also refreshes the related lists. Let that read finish before
    // the next write, so the checks below exercise the panel closing on Trash.
    const refreshedChildren = page.waitForResponse((response) =>
      response.request().method() === 'GET' && new URL(response.url()).pathname === `/v1/tickets/${ticket.id}/children`,
    )
    await page.getByRole('button', { name: 'Unarchive' }).click()
    expect((await refreshedChildren).ok()).toBe(true)
    await expect(page.getByText('This ticket is archived.')).toHaveCount(0)
    await expect(page.getByRole('textbox', { name: 'Title' })).toBeEditable()
    await expect(page.getByRole('button', { name: 'Edit description' })).toBeVisible()

    // No 404 is allowed here: the panel closes before the live refresh of the trash can read the Ticket (TrashTicketButton),
    // and the Ticket is live (or restored) everywhere else in this test, so a refused read of it is a real defect.
    await page.getByRole('button', { name: 'Move to trash' }).click()
    const trashAsk = page.getByRole('alertdialog').or(page.getByRole('dialog').filter({ hasText: 'Move to trash' })).last()
    await expect(trashAsk).toContainText(`Move ${ticket.key} to the trash?`)
    await trashAsk.getByRole('button', { name: 'Move to trash' }).click()
    await expect(page).toHaveURL(new RegExp(`${work}$`))
    await expect(page.getByRole('button', { name: `Open ${ticket.key}` })).toHaveCount(0)
    expect((await api.board(project.id)).columns.flatMap((column) => column.tickets).some((item) => item.id === ticket.id)).toBe(false)

    await page.goto(`/w/${workspaceId}/projects/${project.id}/settings`)
    const trash = page.getByRole('region', { name: 'Trash' })
    await expect(trash).toContainText(ticket.key)
    await trash.getByRole('button', { name: 'Restore' }).click()
    await expect(trash).toContainText('The trash is empty.')
    await page.goto(work)
    await expect(page.getByRole('button', { name: `Open ${ticket.key}` })).toBeVisible()

    // Into the trash again, then gone for good.
    await openPanel()
    await page.getByRole('button', { name: 'Move to trash' }).click()
    await page.getByRole('alertdialog').or(page.getByRole('dialog').filter({ hasText: 'Move to trash' })).last().getByRole('button', { name: 'Move to trash' }).click()
    await page.goto(`/w/${workspaceId}/projects/${project.id}/settings`)
    await trash.getByRole('button', { name: 'Purge' }).click()
    const purgeAsk = page.getByRole('alertdialog').or(page.getByRole('dialog').filter({ hasText: 'Purge' })).last()
    await expect(purgeAsk).toContainText(`Purge ${ticket.key}?`)
    await purgeAsk.getByRole('button', { name: 'Purge' }).click()
    await expect(trash).toContainText('The trash is empty.')
    const gone = await request.fetch(`/v1/tickets/${ticket.id}`, { headers: HEADERS })
    expect(gone.status()).toBe(404)
  })
})

test.describe('incoming webhooks of a channel', () => {
  test('a manager makes a URL-secret webhook in Channel settings, a post to it shows in the channel at once, and a revoked one is refused', async ({ page, request, allowProblem }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `hook-${stamp()}`)
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}?settings=1`)
    const settings = page.getByRole('dialog', { name: 'Channel settings' })
    await settings.getByRole('button', { name: 'Webhooks' }).click()
    const hooks = page.getByRole('dialog', { name: 'Incoming webhooks' })
    await expect(hooks.getByText('This channel has no webhooks.')).toBeVisible()
    await hooks.getByLabel('Name', { exact: true }).fill('Build bot')
    await hooks.getByLabel('Author name', { exact: true }).fill('CI')
    // The server asks for a recent sign-in once (as it does some minutes after logging in): the sign-in dialog opens
    // over this one, and after it the webhook is made and its secret shown here. Closing the sign-in dialog used to
    // close this dialog too, and the secret, shown only once, was lost (v0.2.0).
    allowProblem(/403 POST .*\/webhooks/)
    allowProblem(/Failed to load resource: the server responded with a status of 403/)
    await page.route(/\/v1\/channels\/[^/]+\/webhooks$/, (route) => route.request().method() === 'POST'
      ? route.fulfill({ status: 403, contentType: 'application/json', body: JSON.stringify({ code: 'reauth_required', message: 'recent authentication is required', details: { methods: ['password'] } }) })
      : route.fallback(), { times: 1 })
    await hooks.getByRole('button', { name: 'Create webhook' }).click()
    const again = page.getByRole('dialog', { name: 'Confirm it is you' })
    await again.getByLabel('Password').fill(PASSWORD)
    await again.getByRole('button', { name: 'Confirm' }).click()
    await expect(again).toHaveCount(0)
    await expect(hooks.getByText('This secret is shown only now. It is not in the list.')).toBeVisible()
    const secret = (await hooks.locator('code').innerText()).trim()
    expect(secret.length).toBeGreaterThan(20)
    await hooks.getByRole('button', { name: 'Done' }).click()
    await expect(hooks.getByText(/CI · Active/)).toBeVisible()
    await hooks.getByRole('button', { name: 'Done' }).click()
    await settings.locator('.channel-settings-head').getByRole('button', { name: 'Close' }).click()

    const text = `build ${stamp()} passed`
    const posted = await request.fetch(`/v1/hooks/${secret}`, { method: 'POST', data: { text } })
    expect(posted.status(), await posted.text()).toBeLessThan(300)
    await expect(page.getByRole('article').filter({ hasText: text })).toContainText('CI')

    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}?settings=1`)
    await settings.getByRole('button', { name: 'Webhooks' }).click()
    // Revoking cannot be undone, so it is asked first (v0.2.1); Cancel keeps the webhook working.
    await hooks.getByRole('button', { name: 'Revoke' }).click()
    const ask = page.getByRole('dialog', { name: 'Revoke' })
    await expect(ask).toContainText('Revoke the webhook "Build bot"?')
    await ask.getByRole('button', { name: 'Cancel' }).click()
    await expect(hooks.getByText(/CI · Active/)).toBeVisible()
    await hooks.getByRole('button', { name: 'Revoke' }).click()
    await ask.getByRole('button', { name: 'Revoke' }).click()
    await expect(hooks.getByText(/CI · Revoked/)).toBeVisible()
    const refused = await request.fetch(`/v1/hooks/${secret}`, { method: 'POST', data: { text: 'after revoke' } })
    expect([401, 403, 404]).toContain(refused.status())
  })
})

test.describe('Web Push switch', () => {
  test('is offered when the server has a VAPID key, and a refused browser permission says so and subscribes nothing', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const key = await (await request.fetch('/v1/push/vapid-public-key', { headers: HEADERS })).json().catch(() => null) as { configured?: boolean } | null
    test.skip(!key?.configured, 'the stack has no VAPID key (PENFII_VAPID_PRIVATE_KEY)')
    // The browser's answer to the permission prompt is "denied": no push service is contacted.
    await page.addInitScript(() => {
      Object.defineProperty(Notification, 'permission', { get: () => 'default', configurable: true })
      Notification.requestPermission = async () => 'denied'
    })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/settings/notifications`)
    const region = page.getByRole('region', { name: 'Push notifications' })
    const toggle = region.getByRole('switch', { name: 'Receive push notifications on this device' })
    const unsupported = region.getByText('This browser does not support push notifications.')
    await expect(toggle.or(unsupported).first()).toBeVisible()
    test.skip(await unsupported.isVisible(), 'this engine has no Push API here (service workers are blocked in the test context)')
    await toggle.click()
    await expect(region.getByRole('status')).toContainText('Notifications are blocked in this browser.')
    const devices = (await (await request.fetch('/v1/me/push-subscriptions', { headers: HEADERS })).json()) as { items: unknown[] }
    expect(devices.items).toEqual([])
  })
})

test.describe('the phone layout', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true })

  test('chat, a channel and its settings, a ticket and its description fit the screen and work', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `phone-${stamp()}`)
    const project = await api.createProject(workspaceId, `Phone ${stamp()}`)
    const ticket = await api.createTicket(project.id, `A rather long ticket title to see how a phone shows it ${stamp()}`)
    await logIn(page, accountA.email)

    await page.goto(`/w/${workspaceId}/chat`)
    const tabs = page.getByRole('navigation').filter({ has: page.getByRole('link', { name: 'Chat' }) }).last()
    await expect(tabs.getByRole('link', { name: 'Projects' })).toBeVisible()
    expect(await noSidewaysScroll(page)).toBe(true)

    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    await expect(page.getByRole('heading', { name: `# ${channel.name}`, level: 1 })).toBeVisible()
    const box = page.getByRole('textbox', { name: 'Write a message' })
    await box.click()
    await page.keyboard.type(`from a phone ${channel.name}`)
    expect(await inViewport(page, '[aria-label="Write a message"]')).toBe(true)
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(page.getByRole('article').filter({ hasText: `from a phone ${channel.name}` })).toBeVisible()
    expect(await noSidewaysScroll(page)).toBe(true)

    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}?settings=1`)
    const settings = page.getByRole('dialog', { name: 'Channel settings' })
    await expect(settings).toBeVisible()
    expect(await inViewport(page, '[role="dialog"][aria-label="Channel settings"], [role="dialog"]:has(h2:text("Channel settings"))')).toBe(true)
    await settings.locator('.channel-settings-head').getByRole('button', { name: 'Close' }).click()
    await expect(settings).toHaveCount(0)
    await expect(page.getByRole('heading', { name: `# ${channel.name}`, level: 1 })).toBeVisible()

    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    await expect(page.getByRole('textbox', { name: 'Title' })).toHaveValue(/A rather long ticket title/)
    expect(await noSidewaysScroll(page)).toBe(true)
    await page.getByRole('button', { name: 'Edit description' }).click()
    const editor = page.locator('.ticket-description-blocknote [contenteditable="true"]').first()
    await expect(editor).toBeFocused()
    await page.keyboard.type(`phone description ${channel.name}`)
    const save = page.locator('.btn-primary', { hasText: /^Save$/ })
    await expect(save).toBeVisible()
    await save.click()
    await expect(page.getByRole('button', { name: 'Edit description' })).toBeVisible()
    await expect(page.getByText(`phone description ${channel.name}`)).toBeVisible()
    await expect.poll(async () => JSON.stringify((await api.ticket(ticket.id)).description)).toContain(`phone description ${channel.name}`)
    // Editing again and cancelling keeps what was saved.
    await page.getByRole('button', { name: 'Edit description' }).click()
    await expect(editor).toBeFocused()
    await page.keyboard.press('End')
    await page.keyboard.type(' and more')
    await page.locator('.btn', { hasText: /^Cancel$/ }).click()
    await expect(page.getByText(`phone description ${channel.name} and more`)).toHaveCount(0)
    await expect(page.getByText(`phone description ${channel.name}`)).toBeVisible()
    await page.getByRole('button', { name: '← Back' }).click()
    await expect(page).toHaveURL(new RegExp(`/projects/${project.id}/work$`))
    await expect(page.getByRole('button', { name: `Open ${ticket.key}` })).toBeVisible()
    expect(await noSidewaysScroll(page)).toBe(true)
  })
})

test.describe('a floating menu whose trigger leaves the viewport', () => {
  test('stays inside the viewport when the window shrinks below its trigger', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await api.createChannel(workspaceId, `edge-${stamp()}`)
    await logIn(page, accountA.email)
    await page.addInitScript((id) => {
      localStorage.setItem(`penfii.chat.sections.${id}`, JSON.stringify({ sections: Array.from({ length: 18 }, (_, index) => ({ id: `section-${index}`, name: `Section ${index}`, conversationIds: [] })) }))
    }, workspaceId)
    await page.setViewportSize({ width: 1120, height: 900 })
    await page.goto(`/w/${workspaceId}/chat`)
    const nav = page.locator('.workspace-nav')
    // A trigger low on a tall window, so a shorter window leaves it below the bottom edge.
    const trigger = nav.getByRole('button', { name: 'Conversation menu' }).first()
    await trigger.scrollIntoViewIfNeeded()
    await nav.evaluate((element, target) => {
      const row = element.querySelectorAll('[aria-label="Conversation menu"]')[0] as HTMLElement
      element.scrollTop += row.getBoundingClientRect().top - (target - 40)
    }, 900)
    await trigger.click()
    const menu = page.getByRole('menu')
    await expect(menu).toBeVisible()
    await page.setViewportSize({ width: 1120, height: 560 })
    await expect.poll(() => trigger.evaluate((element) => element.getBoundingClientRect().top > window.innerHeight)).toBe(true)
    await expect.poll(() => menu.evaluate((element) => {
      const rect = element.getBoundingClientRect()
      return rect.top >= 0 && rect.bottom <= window.innerHeight
    })).toBe(true)
    await page.keyboard.press('Escape')
    await expect(menu).toHaveCount(0)
  })
})

