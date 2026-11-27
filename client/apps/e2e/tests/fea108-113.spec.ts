import { randomUUID } from 'node:crypto'
import { expect, test } from '../support/test'
import { accountA } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// FEA-108 to FEA-113: the create-channel form, the ticket draft, the profile bar, search, the document delete question
// and the chat reading width, in the real app.

test.describe('FEA-108: a new channel name', () => {
  test('a blank name is explained under the field, and one request is sent however often Create is pressed', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/chat`)
    await page.getByRole('button', { name: /\+ New/ }).first().click()
    await page.getByRole('menuitem', { name: 'New channel' }).click()
    const name = page.getByLabel('Channel name')
    await name.focus()
    await name.blur()
    await expect(page.getByRole('alert')).toHaveCount(0)
    await name.fill('   ')
    await page.getByRole('button', { name: 'Create channel' }).click()
    await expect(page.getByText('Enter a channel name. A name of only spaces cannot be used.')).toBeVisible()
    await expect(name).toBeFocused()

    let posts = 0
    await page.route(new RegExp(`/v1/workspaces/${workspaceId}/channels$`), async (route) => {
      if (route.request().method() === 'POST') {
        posts += 1
        await new Promise((resolve) => setTimeout(resolve, 1200))
      }
      await route.continue()
    })
    await name.fill(`qa-e2e-${randomUUID().slice(0, 6)}`)
    await page.getByRole('button', { name: 'Create channel' }).dblclick()
    await page.keyboard.press('Escape')
    await expect(page).toHaveURL(/\/c\//, { timeout: 15_000 })
    expect(posts).toBe(1)
  })
})

test.describe('FEA-109: the ticket draft', () => {
  test('Esc keeps the text and Reopen brings it back; with a file attached Esc asks first', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA draft ${randomUUID().slice(0, 4)}`)
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket-new&newProject=${project.id}`)
    const panel = page.getByRole('complementary', { name: 'Create ticket' })
    const title = panel.getByLabel('Title *')
    await title.fill('Half written')
    await expect.poll(() => page.evaluate(() => Object.keys(localStorage).some((key) => key.startsWith('penfii.ticketDraft')))).toBe(true)
    await title.press('Escape')
    await expect(panel).toHaveCount(0)
    await page.getByRole('button', { name: 'Reopen' }).click()
    await expect(panel.getByLabel('Title *')).toHaveValue('Half written')
    await expect(panel.getByRole('status').filter({ hasText: 'Restored the draft' })).toBeVisible()

    await panel.locator('input[type=file]').setInputFiles({ name: 'qa.txt', mimeType: 'text/plain', buffer: Buffer.from('x') })
    await expect(panel.getByText('qa.txt')).toBeVisible()
    await panel.getByLabel('Title *').press('Escape')
    const question = page.getByRole('dialog', { name: 'Attachments are not kept' })
    await expect(question).toBeVisible()
    await expect(question.getByRole('button', { name: 'Keep writing' })).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(panel).toBeVisible()
    await expect(panel.getByText('qa.txt')).toBeVisible()
    await page.evaluate(() => Object.keys(localStorage).filter((key) => key.startsWith('penfii.ticketDraft')).forEach((key) => localStorage.removeItem(key)))
  })
})

test.describe('FEA-110: unsaved profile changes', () => {
  test('a changed name shows the bar and leaving asks, with Keep editing first', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/settings`)
    const name = page.getByLabel(/^Display name/)
    const original = await name.inputValue()
    await expect(page.locator('.profile-savebar')).toHaveCount(0)
    await name.fill(`${original} changed`)
    await expect(page.locator('.profile-savebar')).toContainText('Unsaved changes · 1 items')
    await page.getByRole('link', { name: /Notifications/ }).first().click()
    const question = page.getByRole('dialog', { name: 'You have unsaved changes' })
    await expect(question.getByRole('button', { name: 'Keep editing' })).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(name).toHaveValue(`${original} changed`)
    await page.getByRole('button', { name: 'Revert' }).click()
    await expect(page.locator('.profile-savebar')).toHaveCount(0)
  })
})

test.describe('FEA-111: project filter in the All tab', () => {
  test('a ticket with no channel is found with project: in the All tab, as in the Tickets tab', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA search ${randomUUID().slice(0, 4)}`)
    const word = `qasearch${randomUUID().slice(0, 6)}`
    const ticket = await api.createTicket(project.id, `${word} ticket`)
    const prefix = ticket.key.split('-')[0]
    const find = async (kind: string) => {
      const response = await request.get(`/v1/workspaces/${workspaceId}/search`, { params: { q: `${word} project:${prefix}`, kind, limit: 20, tz: 'UTC' } })
      const body = (await response.json()) as { results: Array<{ kind: string; title: string }> }
      return body.results.filter((hit) => hit.kind === 'ticket').map((hit) => hit.title)
    }
    // The index follows the ticket within a moment.
    await expect.poll(() => find('ticket'), { timeout: 30_000 }).toEqual([`${word} ticket`])
    expect(await find('all')).toEqual([`${word} ticket`])
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/search?q=${encodeURIComponent(`nothing-${word} project:${prefix}`)}&tab=ticket`)
    await expect(page.getByText('Active conditions')).toBeVisible()
    await page.getByRole('button', { name: 'Remove project filter' }).click()
    await expect(page).not.toHaveURL(/project/)
  })
})

test.describe('FEA-112: the document delete question', () => {
  test('says what goes with the page, cancels back to the row, and deletes', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const create = async (title: string, parentId: string | null) => {
      const response = await request.post(`/v1/workspaces/${workspaceId}/docs`, { headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() }, data: { title, visibility: 'WORKSPACE', parentId } })
      expect(response.status()).toBe(201)
      return ((await response.json()) as { id: string }).id
    }
    const tag = randomUUID().slice(0, 5)
    const parent = await create(`QA del ${tag}`, null)
    await create(`QA del child ${tag}`, parent)
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/docs`)
    const row = page.getByRole('treeitem', { name: `QA del ${tag}` })
    await row.focus()
    await page.keyboard.press('Delete')
    const question = page.getByRole('dialog', { name: 'Delete document' })
    await expect(question.getByText(`“QA del ${tag}” and 1 subpage(s) will be deleted.`)).toBeVisible()
    await expect(question.getByText('A deleted document cannot be restored in this product.')).toBeVisible()
    await expect(question.getByRole('button', { name: 'Cancel' })).toBeFocused()
    await page.keyboard.press('Escape')
    await expect(row).toBeFocused()
    await row.focus()
    await page.keyboard.press('Delete')
    await question.getByRole('button', { name: 'Delete', exact: true }).click()
    await expect(page.getByText(`Deleted “QA del ${tag}” and 1 subpage(s).`)).toBeVisible()
    await expect(row).toHaveCount(0)
  })
})

test.describe('FEA-113: chat reading width', () => {
  test('on a 2560px window, with no thread open, the messages and the composer share one centred block that keeps 32px clear on each side with no fixed cap, and the typed text starts where the message text starts', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `qa-wide-${randomUUID().slice(0, 5)}`)
    await page.setViewportSize({ width: 2560, height: 1100 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    const editor = page.locator('.conversation > .column:not(.thread-column) .composer .ProseMirror')
    await editor.click()
    await page.keyboard.type('A long line for reading width. '.repeat(40))
    await page.keyboard.press('Enter')
    await expect(page.locator('.message').last()).toBeVisible()
    const column = page.locator('.conversation > .column:not(.thread-column)')
    await expect(column.locator('.composer')).toBeVisible()
    const box = async (selector: string) => {
      const found = await column.locator(selector).first().boundingBox()
      expect(found, selector).not.toBeNull()
      return found!
    }
    const c = await box('.transcript')
    const b = await box('.scroller > div')
    const k = await box('.composer')
    const geometry = {
      columnWidth: c.width, columnMid: c.x + c.width / 2, blockMid: b.x + b.width / 2, blockWidth: b.width, composerLeft: k.x, composerRight: k.x + k.width, blockLeft: b.x, blockRight: b.x + b.width, composerWidth: k.width,
      hscroll: await page.evaluate(() => document.documentElement.scrollWidth > innerWidth),
    }
    // The block is the column less 32px on each side, with no fixed cap (it was at most 1120px).
    expect(Math.abs(geometry.blockWidth - (geometry.columnWidth - 64))).toBeLessThan(2)
    expect(geometry.blockWidth).toBeGreaterThan(2000)
    expect(Math.abs(geometry.blockMid - geometry.columnMid)).toBeLessThan(2)
    // The composer sits inside the block, never wider than it.
    expect(geometry.composerLeft).toBeGreaterThanOrEqual(geometry.blockLeft)
    expect(geometry.composerRight).toBeLessThanOrEqual(geometry.blockRight)
    // The letters, not the boxes: the first character of the message and the first typed character share a left edge.
    await editor.click()
    await page.keyboard.type('X')
    const starts = await page.evaluate(() => {
      const first = (element: Element) => {
        const node = document.createTreeWalker(element, NodeFilter.SHOW_TEXT).nextNode()!
        const range = document.createRange()
        range.setStart(node, 0)
        range.setEnd(node, 1)
        return range.getBoundingClientRect().left
      }
      const column = document.querySelector('.conversation > .column:not(.thread-column)')!
      const paragraphs = [...column.querySelectorAll('.message-main p')]
      return { body: first(paragraphs[paragraphs.length - 1]), typed: first(column.querySelector('.composer .ProseMirror p')!) }
    })
    expect(Math.abs(starts.body - starts.typed)).toBeLessThan(1)
    expect(geometry.hscroll).toBe(false)
    // A narrower window keeps the same rule: the block is the column less 32px on each side.
    await page.setViewportSize({ width: 1024, height: 800 })
    const narrow = await page.evaluate(() => {
      const column = document.querySelector('.conversation > .column:not(.thread-column)') as HTMLElement
      const block = column.querySelector('.scroller > div') as HTMLElement
      return { column: column.getBoundingClientRect().width, block: block.getBoundingClientRect().width }
    })
    expect(Math.abs(narrow.column - 64 - narrow.block)).toBeLessThan(2)
  })
})

test.describe('Chat width beside the thread', () => {
  test('the block keeps 32px clear on each side of the main column whether a thread is open or not, with no fixed cap', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `qa-thread-width-${randomUUID().slice(0, 5)}`)
    const body = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Width follows the thread. '.repeat(30) }] }] }
    const sent = await request.post(`/v1/conversations/${channel.conversationId}/messages`, {
      headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() },
      data: { bodySchemaVersion: 1, body, threadRootId: null, replyToId: null },
    })
    expect(sent.status()).toBe(201)
    await page.setViewportSize({ width: 1920, height: 900 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    const block = () => page.evaluate(() => {
      const column = document.querySelector('.conversation > .column:not(.thread-column)') as HTMLElement
      const first = column.querySelector('.scroller > div') as HTMLElement
      const composer = column.querySelector('.composer') as HTMLElement
      const c = column.getBoundingClientRect()
      const b = first.getBoundingClientRect()
      const k = composer.getBoundingClientRect()
      return { block: Math.round(b.width), column: Math.round(c.width), composerInside: k.left >= b.left - 1 && k.right <= b.right + 1, centred: Math.abs(b.left + b.width / 2 - (c.left + c.width / 2)) < 2, hscroll: document.documentElement.scrollWidth > innerWidth }
    })
    await expect(page.locator('.message').last()).toBeVisible()
    const closed = await block()
    expect(closed.block).toBe(closed.column - 64)
    expect(closed.block).toBeGreaterThan(1120)
    expect(closed.centred && closed.composerInside && !closed.hscroll).toBe(true)
    await page.locator('.message').last().hover()
    await page.locator('.message').last().getByRole('link', { name: 'Thread' }).click()
    await expect(page.locator('.thread-column')).toBeVisible()
    await expect.poll(async () => { const now = await block(); return now.column < closed.column && now.block === now.column - 64 }).toBe(true)
    const open = await block()
    // 1920 with the default 420px thread: the main column is 1176 and the block 1112 (it was 800).
    expect(open.block).toBeGreaterThan(800)
    expect(open.centred && open.composerInside && !open.hscroll).toBe(true)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    await expect(page.locator('.message').last()).toBeVisible()
    await expect.poll(async () => (await block()).block).toBe(closed.block)
  })
})

test.describe('FEA-113: the bottom of the list stays when the width changes', () => {
  test('opening the thread and narrowing the window keeps a reader who was at the bottom there', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `qa-bottom-${randomUUID().slice(0, 5)}`)
    for (let i = 0; i < 25; i += 1) {
      const body = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: `QA scroll message ${i} ${'A line long enough to wrap when the column gets narrow. '.repeat(6)}` }] }] }
      const response = await request.post(`/v1/conversations/${channel.conversationId}/messages`, {
        headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() },
        data: { bodySchemaVersion: 1, body, threadRootId: null, replyToId: null },
      })
      expect(response.status()).toBe(201)
    }
    await page.setViewportSize({ width: 1440, height: 800 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    const messages = page.locator('.message')
    await expect(messages.last()).toBeVisible()
    const distance = () => page.evaluate(() => {
      const scroller = document.querySelector('.conversation > .column:not(.thread-column) .scroller') as HTMLElement
      return Math.round(scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight)
    })
    await expect.poll(distance).toBeLessThanOrEqual(2)
    await messages.last().hover()
    await messages.last().getByRole('link', { name: 'Thread' }).click()
    await expect(page.locator('.thread-column')).toBeVisible()
    await expect.poll(distance).toBeLessThanOrEqual(2)
    await page.setViewportSize({ width: 1000, height: 800 })
    await expect.poll(distance).toBeLessThanOrEqual(2)
    await page.setViewportSize({ width: 1440, height: 800 })
    await expect.poll(distance).toBeLessThanOrEqual(2)
  })
})

test.describe('FEA-111: on a phone, another kind keeps the search', () => {
  test('back from the results lands on the search list with the same text, and another kind searches it again', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await page.setViewportSize({ width: 390, height: 780 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/search`)
    const box = page.getByRole('searchbox', { name: 'Search' })
    await box.fill('qa-phone-kind')
    await box.press('Enter')
    await expect(page).toHaveURL(/q=qa-phone-kind/)
    await page.getByRole('button', { name: /^Back to/ }).click()
    await expect(page.getByRole('searchbox', { name: 'Search' })).toHaveValue('qa-phone-kind')
    await page.getByRole('button', { name: 'Tickets' }).click()
    await page.getByRole('searchbox', { name: 'Search' }).press('Enter')
    await expect(page).toHaveURL(/tab=ticket/)
    await expect(page).toHaveURL(/q=qa-phone-kind/)
  })
})
