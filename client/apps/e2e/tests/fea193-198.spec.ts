import { randomUUID } from 'node:crypto'
import { request as newRequest, type APIRequestContext, type Page } from '@playwright/test'
import { expect, test } from '../support/test'
import { accountA, accountB } from '../support/accounts'
import { apiAs, logIn, signedInContext } from '../support/api'

// FEA-193 to FEA-198 in the real app: comments in the rich text editor, the sign-in page (focus after a failure, a person who is
// already signed in), a conversation that cannot be opened, and an open Doc page that follows other people's changes.

const HEADERS = { 'x-penfii-client': '1' }

async function call<T = unknown>(request: APIRequestContext, method: string, path: string, data?: unknown): Promise<T> {
  const response = await request.fetch(path, { method, headers: { ...HEADERS, 'idempotency-key': randomUUID() }, data })
  expect(response.status(), `${method} ${path}: ${await response.text()}`).toBeLessThan(300)
  const text = await response.text()
  return (text ? JSON.parse(text) : undefined) as T
}

/** Who an account is (its id), asked as that person. */
async function personOf(email: string): Promise<{ id: string }> {
  const context = await newRequest.newContext({ baseURL: process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080' })
  try {
    return await (await apiAs(context, email)).me()
  } finally {
    await context.dispose()
  }
}

const text = (value: string, marks?: unknown[]) => ({ type: 'text', text: value, ...(marks ? { marks } : {}) })
const paragraph = (...content: unknown[]) => ({ type: 'paragraph', content })
const activeLabel = (page: Page) => page.evaluate(() => {
  const element = document.activeElement
  return element ? `${element.tagName.toLowerCase()}[name=${element.getAttribute('name') ?? ''}]${element.getAttribute('type') === 'submit' ? '[submit]' : ''}` : null
})

test.describe('FEA-194: the keyboard after a sign-in that did not work', () => {
  test('goes to the password, the code or the button that tries again, keeps what was typed, and does not send twice', async ({ page, allowProblem }) => {
    allowProblem(/401 POST .*\/v1\/session\/login|Failed to load resource|request failed: POST .*\/v1\/session\/login|503 POST/)
    await page.goto('/login')
    await page.getByLabel('Email').fill(accountA.email)
    await page.getByLabel('Password').fill('not-the-password')
    await page.getByRole('button', { name: /^Log in/ }).click()
    const alert = page.getByRole('alert')
    await expect(alert).toHaveText('The email or password is incorrect')
    await expect.poll(() => activeLabel(page)).toBe('input[name=password]')
    await expect(page.getByLabel('Password')).toHaveValue('not-the-password')
    await expect(page.getByLabel('Email')).toHaveValue(accountA.email)
    await expect(page.getByLabel('Password')).toHaveAttribute('aria-describedby', (await alert.getAttribute('id')) ?? '')
    // Enter on the field tries again, and the focus is back after that one too.
    await page.keyboard.press('Enter')
    await expect(alert).toHaveText('The email or password is incorrect')
    await expect.poll(() => activeLabel(page)).toBe('input[name=password]')

    // The network fails: the button that sends the form again, never sent by itself.
    let sent = 0
    await page.route('**/v1/session/login', (route) => { sent += 1; void route.abort('failed') })
    await page.getByRole('button', { name: /^Log in/ }).click()
    await expect(alert).toHaveText('The request could not be completed')
    await expect.poll(() => activeLabel(page)).toBe('button[name=][submit]')
    await page.waitForTimeout(500)
    expect(sent).toBe(1)

    // The second step: the code field when it is asked for, and again when it is refused.
    await page.unroute('**/v1/session/login')
    await page.route('**/v1/session/login', (route) => {
      const body = JSON.parse(route.request().postData() ?? '{}') as { code?: string }
      void route.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ code: body.code === undefined ? 'mfa_required' : 'unauthenticated', message: 'x' }) })
    })
    await page.getByRole('button', { name: /^Log in/ }).click()
    const code = page.getByLabel('Authenticator or recovery code')
    await expect(code).toBeFocused()
    await code.fill('000000')
    await page.keyboard.press('Enter')
    await expect(alert).toHaveText('The code is incorrect or was already used')
    await expect(code).toBeFocused()
    await expect(code).toHaveValue('000000')
    await expect(page.getByLabel('Password')).toHaveValue('not-the-password')
  })

  test('leaves a control the person moved to while the request was under way', async ({ page, allowProblem }) => {
    allowProblem(/401 POST .*\/v1\/session\/login|Failed to load resource/)
    await page.goto('/login')
    await page.getByLabel('Email').fill(accountA.email)
    await page.getByLabel('Password').fill('not-the-password')
    let release: () => void = () => undefined
    await page.route('**/v1/session/login', async (route) => {
      await new Promise<void>((resolve) => { release = resolve })
      await route.fulfill({ status: 401, contentType: 'application/json', body: '{"code":"unauthenticated","message":"x"}' })
    })
    await page.getByRole('button', { name: /^Log in/ }).click()
    await expect(page.getByRole('button', { name: /^Log in/ })).toBeDisabled()
    await page.getByLabel('Email').focus()
    release()
    await expect(page.getByRole('alert')).toHaveText('The email or password is incorrect')
    await expect(page.getByLabel('Email')).toBeFocused()
  })
})

test.describe('FEA-195: the sign-in page for a person who is signed in', () => {
  test('takes them on without showing the form, to the remembered place, once, and Back does not return to it', async ({ page }) => {
    await logIn(page, accountA.email)
    const user = await (await page.request.get('/v1/me')).json() as { id: string }
    const workspaceId = (await (await page.request.get('/v1/workspaces')).json() as Array<{ id: string }>)[0].id
    await page.goto(`/w/${workspaceId}/inbox`)
    await page.evaluate(({ path, userId }) => sessionStorage.setItem('penfii.afterLogin', JSON.stringify({ path, userId })), { path: `/w/${workspaceId}/docs`, userId: user.id })
    let formSeen = false
    await page.exposeFunction('formWasShown', () => { formSeen = true })
    await page.addInitScript(() => {
      new MutationObserver(() => { if (document.querySelector('#login-email')) void (window as unknown as { formWasShown: () => void }).formWasShown() }).observe(document, { childList: true, subtree: true })
    })
    await page.goto('/login')
    await expect(page).toHaveURL(new RegExp(`/w/${workspaceId}/docs$`))
    expect(formSeen).toBe(false)
    expect(await page.evaluate(() => sessionStorage.getItem('penfii.afterLogin'))).toBeNull()
    await page.goBack()
    await expect(page).toHaveURL(new RegExp(`/w/${workspaceId}/inbox$`))

    // Not followed: another person's path, another site, a path that leaves the app, the sign-in page itself.
    for (const path of [`/w/${workspaceId}/projects/not-mine`, '//evil.example/x', '/\\evil.example', '/login']) {
      await page.evaluate(({ stored, userId }) => sessionStorage.setItem('penfii.afterLogin', JSON.stringify({ path: stored, userId })), { stored: path, userId: path.startsWith('/w/') ? 'someone-else' : user.id })
      await page.goto('/login')
      await expect(page).toHaveURL(new RegExp(`/w/${workspaceId}$`))
      expect(new URL(page.url()).hostname).toBe('127.0.0.1')
    }
  })

  test('a signed-out visit is exactly one 401 on /v1/me and nothing else fails', async ({ page }) => {
    // The page asks the server whether a session exists; for a signed-out person the answer is 401. That is all the watcher is
    // told to expect (see support/test.ts): any other failure on the sign-in page still fails the test.
    const failed: string[] = []
    page.on('response', (response) => { if (response.status() >= 400) failed.push(`${response.status()} ${new URL(response.url()).pathname}`) })
    await page.goto('/login')
    await expect(page.getByLabel('Email')).toBeVisible()
    await page.waitForTimeout(500)
    expect(failed).toEqual(['401 /v1/me'])
  })

  test('shows the form for a signed-out person, and says so when the server cannot tell', async ({ page, allowProblem }) => {
    allowProblem(/503 GET .*\/v1\/me|Failed to load resource/)
    await page.route('**/v1/me', (route) => route.fulfill({ status: 503, contentType: 'application/json', body: '{"code":"db_unavailable","message":"x"}' }))
    await page.goto('/login')
    await expect(page.getByRole('alert')).toContainText('The database is temporarily unavailable')
    await expect(page.getByLabel('Email')).toHaveCount(0)
    await page.unroute('**/v1/me')
    await page.getByRole('button', { name: 'Try again' }).click()
    await expect(page.getByLabel('Email')).toBeVisible()
  })
})

test.describe('FEA-196: a conversation that cannot be opened', () => {
  test('says what happened and where to go, for a conversation and for a thread', async ({ page, request, allowProblem }) => {
    allowProblem(/404 GET |Failed to load resource/)
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await logIn(page, accountA.email)
    const channel = await api.createChannel(workspaceId, `qa-unavailable-${randomUUID().slice(0, 6)}`)
    await page.goto(`/w/${workspaceId}/c/${randomUUID()}`)
    await expect(page.getByRole('heading', { name: 'This conversation cannot be opened' })).toBeVisible()
    await expect(page.getByText('It was deleted, or you do not have access to it.')).toBeVisible()
    await page.getByRole('link', { name: 'Back to Chat' }).click()
    await expect(page).toHaveURL(new RegExp(`/w/${workspaceId}/chat$`))

    // A thread that is gone leaves the conversation alone.
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}/t/${randomUUID()}`)
    await expect(page.getByRole('heading', { name: 'This thread cannot be opened' })).toBeVisible()
    await expect(page.getByRole('heading', { name: new RegExp(`# ${channel.name}`) })).toBeVisible()
    await page.getByRole('link', { name: 'Back to the conversation' }).click()
    await expect(page).toHaveURL(new RegExp(`/w/${workspaceId}/c/${channel.conversationId}$`))
    await expect(page.getByRole('heading', { name: 'This thread cannot be opened' })).toHaveCount(0)
  })

  test('a person who is removed from a private channel while it is open sees it, and its messages and box go away', async ({ browser, request, watchPage, allowProblem }) => {
    // The server refuses the removed member's reads, and also a reconnect of the conversation socket if the page makes
    // one before it learns of the removal (a 404 handshake; whether that happens first is timing).
    allowProblem(/40[34] |Failed to load resource|WebSocket handshake: Unexpected response code: 404/)
    const owner = await apiAs(request, accountA.email)
    const workspaceId = await owner.workspaceId()
    const channel = await owner.createChannel(workspaceId, `qa-private-${randomUUID().slice(0, 6)}`, 'PRIVATE')
    const member = await personOf(accountB.email)
    await owner.addChannelMember(channel.id, member.id)
    const { context, page } = await signedInContext(browser, accountB.email, watchPage)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    await expect(page.getByRole('heading', { name: new RegExp(`# ${channel.name}`) })).toBeVisible()
    await call(request, 'DELETE', `/v1/channels/${channel.id}/members/${member.id}`)
    await expect(page.getByRole('heading', { name: 'This conversation cannot be opened' })).toBeVisible()
    await expect(page.getByRole('textbox', { name: /Message|Write/ })).toHaveCount(0)
    await context.close()
  })
})

test.describe('FEA-193: comments are written and edited in the rich text editor', () => {
  test('a ticket comment with headings, lists, a quote and code keeps all of it when one word is changed, and Escape leaves the edit', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA comments ${randomUUID().slice(0, 5)}`)
    const ticket = await api.createTicket(project.id, 'Comment round trip')
    const stored = {
      type: 'doc',
      content: [
        { type: 'heading', attrs: { level: 2 }, content: [text('Plan')] },
        paragraph(text('first '), text('bold', [{ type: 'bold' }]), text(' and '), text('link', [{ type: 'link', attrs: { href: 'https://example.com/spec' } }])),
        { type: 'orderedList', attrs: { start: 3 }, content: [
          { type: 'listItem', content: [paragraph(text('three')), { type: 'bulletList', content: [{ type: 'listItem', content: [paragraph(text('nested'))] }] }] },
          { type: 'listItem', content: [paragraph(text('four'))] },
        ] },
        { type: 'blockquote', content: [paragraph(text('quoted'))] },
        { type: 'codeBlock', attrs: { language: 'rust' }, content: [text('fn main() {}')] },
      ],
    }
    await call(request, 'POST', `/v1/tickets/${ticket.id}/comments`, { body: stored, fileIds: [] })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    const panel = page.getByRole('complementary', { name: 'Ticket detail' })
    await panel.getByRole('button', { name: /^Comments/ }).click()
    await expect(panel.getByRole('heading', { name: 'Plan' })).toBeVisible()
    await panel.getByRole('button', { name: 'Edit', exact: true }).click()
    const box = panel.getByRole('textbox', { name: 'Edit', exact: true })
    await expect(box.locator('h2')).toHaveText('Plan')
    await expect(box.locator('ol')).toHaveAttribute('start', '3')
    await expect(box).toBeFocused()

    // Save without a change sends nothing: the comment stays "not edited".
    let patches = 0
    await page.route('**/v1/tickets/*/comments/*', (route) => { if (route.request().method() === 'PATCH') patches += 1; void route.continue() })
    await panel.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(box).toHaveCount(0)
    expect(patches).toBe(0)
    await expect(panel.getByRole('button', { name: 'Edit', exact: true })).toBeFocused()

    // Escape leaves an edit and does not close the panel; a change saves with Ctrl+Enter.
    await panel.getByRole('button', { name: 'Edit', exact: true }).click()
    await expect(box).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(box).toHaveCount(0)
    await expect(panel).toBeVisible()
    await panel.getByRole('button', { name: 'Edit', exact: true }).click()
    await box.getByText('four').click()
    await page.keyboard.press('End')
    await page.keyboard.type('!')
    await page.keyboard.press('Control+Enter')
    await expect(box).toHaveCount(0)
    expect(patches).toBe(1)
    type Stored = { body: { content: unknown[] }; editedAt: string | null }
    const read = await call<Stored[] | { items: Stored[] }>(request, 'GET', `/v1/tickets/${ticket.id}/comments`)
    const after = { items: Array.isArray(read) ? read : read.items }
    const body = JSON.parse(JSON.stringify(after.items[0].body)) as { content: Array<{ content?: Array<{ content: Array<{ content: Array<{ text: string }> }> }> }> }
    expect(body.content[2].content![1].content[0].content[0].text).toBe('four!')
    // Everything else is as it was stored.
    body.content[2].content![1].content[0].content[0].text = 'four'
    expect(body).toEqual(stored)
    expect(after.items[0].editedAt).not.toBeNull()

    // With the keyboard in the comment box, Escape closes the panel (as it did with a plain box).
    await panel.getByRole('textbox', { name: 'Write a comment' }).click()
    await page.keyboard.press('Escape')
    await expect(panel).toHaveCount(0)
  })

  test('a new ticket comment is written with formatting and posts with Ctrl+Enter, Enter breaking the line', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA comments ${randomUUID().slice(0, 5)}`)
    const ticket = await api.createTicket(project.id, 'Comment writing')
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    const panel = page.getByRole('complementary', { name: 'Ticket detail' })
    const box = panel.getByRole('textbox', { name: 'Write a comment' })
    await expect(box).toBeFocused()
    await page.keyboard.type('## Heading')
    await page.keyboard.press('Enter')
    await page.keyboard.type('some ')
    await page.keyboard.press('Control+b')
    await page.keyboard.type('bold')
    await page.keyboard.press('Control+b')
    await page.keyboard.press('Enter')
    await page.keyboard.type('- one')
    await page.keyboard.press('Enter')
    await page.keyboard.type('two')
    await expect(panel.locator('.comment-item')).toHaveCount(0)
    await page.keyboard.press('Control+Enter')
    await panel.getByRole('button', { name: /^Comments/ }).click()
    await expect(panel.locator('.comment-text h2')).toHaveText('Heading')
    await expect(panel.locator('.comment-text strong')).toHaveText('bold')
    await expect(panel.locator('.comment-text li')).toHaveCount(2)
    await expect(box).toHaveText('')
  })

  test('a Doc comment keeps its mention (same occurrence, nobody told twice) when its text is changed', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const doc = await api.createDoc(workspaceId, `QA doc comment ${randomUUID().slice(0, 5)}`)
    const bravo = await personOf(accountB.email)
    const occurrenceId = randomUUID()
    await call(request, 'POST', `/v1/docs/${doc.id}/comments`, { body: { type: 'doc', content: [paragraph({ type: 'mention', attrs: { occurrenceId, kind: 'USER', userId: bravo.id } }, text(' please check, and not @anyone'))] } })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
    const comments = page.locator('.doc-comments')
    await expect(comments.locator('.mention')).toContainText('@')
    await comments.getByRole('button', { name: 'Edit', exact: true }).click()
    const box = comments.getByRole('textbox', { name: 'Edit', exact: true })
    await expect(box.locator('.mention')).toHaveCount(1)
    await box.click()
    await page.keyboard.press('End')
    await page.keyboard.type(' thanks')
    await page.keyboard.press('Control+Enter')
    await expect(box).toHaveCount(0)
    const list = await call<{ items: Array<{ body: { content: Array<{ content: Array<{ type: string; attrs?: { occurrenceId: string; userId: string }; text?: string }> }> } }> }>(request, 'GET', `/v1/docs/${doc.id}/comments`)
    const inline = list.items[0].body.content[0].content
    expect(inline.filter((node) => node.type === 'mention')).toEqual([{ type: 'mention', attrs: { occurrenceId, kind: 'USER', userId: bravo.id } }])
    expect(inline.map((node) => node.text ?? '').join('')).toContain('not @anyone thanks')
  })
})

test.describe('FEA-197: an open Doc page follows other people', () => {
  test('a reader sees a Save, the title, the width and a Version at once, and loses the page when it is taken away', async ({ browser, request, watchPage, allowProblem }) => {
    allowProblem(/404 GET .*\/v1\/docs\/|Failed to load resource/)
    const owner = await apiAs(request, accountA.email)
    const workspaceId = await owner.workspaceId()
    const doc = await owner.createDoc(workspaceId, `QA live ${randomUUID().slice(0, 5)}`)
    const { context, page: reader } = await signedInContext(browser, accountB.email, watchPage)
    await reader.setViewportSize({ width: 1400, height: 900 })
    await reader.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
    await expect(reader.getByRole('heading', { level: 1, name: /QA live/ })).toBeVisible()
    let reads = 0
    reader.on('request', (r) => { if (new RegExp(`/v1/docs/${doc.id}$`).test(new URL(r.url()).pathname) && r.method() === 'GET') reads += 1 })
    // Give the page's socket a moment to open, so the first Save already has somebody to tell.
    await reader.waitForTimeout(1500)

    // A Save by somebody else: eight in a row are read once.
    const content = (words: string) => ({ schema: 'penfii.doc', version: 1, blocks: [{ id: 'one', type: 'paragraph', props: {}, content: [{ type: 'text', text: words, styles: {} }], children: [] }] })
    let revision = doc.revisionNo
    const baseline = reads
    for (let n = 1; n <= 8; n += 1) {
      const saved = await call<{ revisionNo: number }>(request, 'PUT', `/v1/docs/${doc.id}/content`, { expectedRevisionNo: revision, title: `QA live renamed ${n}`, content: content(`saved by the owner, ${n}`) })
      revision = saved.revisionNo
    }
    await expect(reader.getByText('saved by the owner, 8')).toBeVisible()
    await expect(reader.getByRole('heading', { level: 1, name: 'QA live renamed 8' })).toBeVisible()
    await reader.waitForTimeout(1500)
    expect(reads - baseline).toBeLessThanOrEqual(2)

    // The width, which has no revision.
    await call(request, 'PATCH', `/v1/docs/${doc.id}`, { fullWidth: true })
    await expect(reader.locator('article.doc-page')).toHaveClass(/is-wide/)
    await call(request, 'PATCH', `/v1/docs/${doc.id}`, { fullWidth: false })
    await expect(reader.locator('article.doc-page')).not.toHaveClass(/is-wide/)

    // A Version changes the line under the title.
    await call(request, 'POST', `/v1/docs/${doc.id}/versions`, { expectedRevisionNo: revision })
    await expect(reader.getByText(/Latest Version 1/)).toBeVisible()

    // Access taken away: nothing of the page stays.
    await call(request, 'PUT', `/v1/docs/${doc.id}/visibility`, { visibility: 'RESTRICTED' })
    await expect(reader.getByRole('heading', { name: 'This page cannot be opened' })).toBeVisible()
    await expect(reader.getByText('saved by the owner, 8')).toHaveCount(0)
    await context.close()
  })

  test('a draft being edited is not touched by other people’s saves, and Save keeps the conflict notice', async ({ browser, request, watchPage, allowProblem }) => {
    allowProblem(/409 PUT .*\/content|Failed to load resource/)
    const owner = await apiAs(request, accountA.email)
    const workspaceId = await owner.workspaceId()
    const doc = await owner.createDoc(workspaceId, `QA draft ${randomUUID().slice(0, 5)}`)
    const { context, page: editor } = await signedInContext(browser, accountB.email, watchPage)
    await editor.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
    await editor.getByRole('button', { name: 'Edit', exact: true }).click()
    const surface = editor.locator('.bn-editor[contenteditable=true]').first()
    await surface.click()
    await editor.keyboard.type('my own draft text')
    const content = { schema: 'penfii.doc', version: 1, blocks: [{ id: 'one', type: 'paragraph', props: {}, content: [{ type: 'text', text: 'the owner saved this meanwhile', styles: {} }], children: [] }] }
    await call(request, 'PUT', `/v1/docs/${doc.id}/content`, { expectedRevisionNo: doc.revisionNo, title: `QA draft ${randomUUID().slice(0, 5)}`, content })
    await editor.waitForTimeout(2500)
    await expect(surface).toContainText('my own draft text')
    await expect(surface).not.toContainText('the owner saved this meanwhile')
    await editor.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(editor.getByText(/Someone saved this page first/)).toBeVisible()
    await expect(surface).toContainText('my own draft text')
    await context.close()
  })
})

test.describe('FEA-198: the ticket panels across window widths', () => {
  test('a ticket being created keeps its title and URL across every boundary, and the panel overlays or splits by the existing rules', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA widths ${randomUUID().slice(0, 5)}`)
    await api.createTicket(project.id, 'A ticket on the board')
    await logIn(page, accountA.email)
    await page.setViewportSize({ width: 1400, height: 900 })
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket-new&newProject=${project.id}`)
    const title = page.getByRole('complementary', { name: 'Create ticket' }).getByRole('textbox', { name: /^Title\b/ })
    await expect(title).toBeVisible()
    await title.fill('Draft that must survive')
    const url = page.url()
    // 1181 and up: a column beside the board; 1180 down to 961: floats over it; 960 and down: the whole width.
    for (const width of [1181, 1180, 961, 960, 768, 390, 768, 961, 1181, 1400]) {
      await page.setViewportSize({ width, height: 900 })
      await expect(title).toHaveValue('Draft that must survive')
      expect(page.url()).toBe(url)
      expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), `no sideways scroll at ${width}`).toBe(false)
      const position = await page.locator('aside[aria-label="Create ticket"]').evaluate((element) => getComputedStyle(element).position)
      expect(position, `panel position at ${width}`).toBe(width > 1180 ? 'relative' : 'fixed')
    }
  })
})
