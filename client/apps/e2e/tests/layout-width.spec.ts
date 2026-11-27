import { randomUUID } from 'node:crypto'
import { expect, test } from '../support/test'
import { accountA } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// How Docs and Chat use the room they have: the Doc page width, the chat block, and a thread wider than the room beside it.

test.describe('a document uses the room it has', () => {
  test('by default the page takes 88% of its column, at full width all but 32px on each side, and every part shares one edge', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const doc = await api.createDoc(workspaceId, `QA room ${randomUUID().slice(0, 5)}`)
    await logIn(page, accountA.email)
    const geometry = () => page.evaluate(() => {
      const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect()
      const column = box('.doc-body > .doc-column')
      const parts = ['.doc-head', '.doc-comments', '.doc-comments .comment-form'].map((selector) => box(selector))
      return {
        column: column.width,
        left: parts[0].left - column.left,
        right: column.right - parts[0].right,
        sameEdges: parts.every((part) => Math.abs(part.left - parts[0].left) < 1 && Math.abs(part.right - parts[0].right) < 1),
        hscroll: document.documentElement.scrollWidth > innerWidth,
      }
    })
    for (const width of [1920, 2560, 1280]) {
      await page.setViewportSize({ width, height: 1000 })
      await page.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
      await expect(page.locator('.doc-head').first()).toBeVisible()
      await expect(page.locator('.doc-comments .comment-form')).toBeVisible()
      const shown = await geometry()
      const side = Math.max(32, shown.column * 0.06)
      expect(Math.abs(shown.left - side), `left side at ${width}`).toBeLessThan(1.5)
      expect(Math.abs(shown.right - side), `right side at ${width}`).toBeLessThan(1.5)
      expect(shown.sameEdges && !shown.hscroll).toBe(true)
    }
    // 1920: the column between the navigation panel and the outline is 1316, so the page is 1158 (it was 632).
    await page.setViewportSize({ width: 1920, height: 1000 })
    await page.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
    await expect(page.locator('.doc-head').first()).toBeVisible()
    expect(Math.round((await geometry()).column)).toBe(1316)
    const changed = await request.patch(`/v1/docs/${doc.id}`, { headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() }, data: { fullWidth: true } })
    expect(changed.status()).toBe(200)
    await page.reload()
    await expect(page.locator('.doc-page.is-wide')).toBeVisible()
    const wide = await geometry()
    expect(Math.round(wide.left)).toBe(32)
    expect(Math.round(wide.right)).toBe(32)
    expect(wide.sameEdges && !wide.hscroll).toBe(true)
  })
})

test.describe('a wide thread scrolls the conversation sideways', () => {
  test('the nav steps aside first, the main column keeps 640px, a drag keeps the handle under the pointer, and the sideways scroll is otherwise left to the reader', async ({ page, request, browserName }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `qa-sideways-${randomUUID().slice(0, 5)}`)
    const body = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Sideways scroll beside a wide thread. '.repeat(10) }] }] }
    const sent = await request.post(`/v1/conversations/${channel.conversationId}/messages`, {
      headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() },
      data: { bodySchemaVersion: 1, body, threadRootId: null, replyToId: null },
    })
    expect(sent.status()).toBe(201)
    const root = (await sent.json()) as { id: string }
    await page.setViewportSize({ width: 1920, height: 1000 })
    await logIn(page, accountA.email)
    await page.evaluate(() => { localStorage.setItem('penfii.threadWidth', '420'); localStorage.removeItem('penfii.contextNavWidth') })
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}/t/${root.id}`)
    await expect(page.locator('.thread-column')).toBeVisible()
    const main = page.locator('.conversation > .column:not(.thread-column) .composer .ProseMirror')
    await main.click()
    await page.keyboard.type('draft beside the thread')
    const state = () => page.evaluate(() => {
      const conversation = document.querySelector('.conversation')!
      const handle = document.querySelector('.thread-column .resize-handle')!.getBoundingClientRect()
      return {
        thread: Math.round(document.querySelector('.thread-column')!.getBoundingClientRect().width),
        main: Math.round(document.querySelector('.conversation > .column:not(.thread-column)')!.getBoundingClientRect().width),
        overflow: conversation.scrollWidth - conversation.clientWidth,
        scrollLeft: Math.round(conversation.scrollLeft),
        handle: Math.round(handle.left + handle.width / 2),
        navCollapsed: document.querySelector('.product-shell')!.classList.contains('nav-collapsed'),
        hscroll: document.documentElement.scrollWidth > innerWidth,
      }
    })
    expect(await state()).toMatchObject({ thread: 420, overflow: 0, navCollapsed: false })

    // While resizing: drag the thread to 1300px. The nav steps aside (its "Show sidebar" tab keeps a 24px strip), then the main column
    // stops at 640 and the rest scrolls.
    const box = (await page.locator('.thread-column .resize-handle').boundingBox())!
    const x = box.x + box.width / 2
    const y = box.y + 300
    await page.mouse.move(x, y)
    await page.mouse.down()
    await page.mouse.move(x - 500, y, { steps: 8 })
    const middle = await state()
    expect(middle.navCollapsed).toBe(true)
    expect(Math.abs(middle.handle - (x - 500))).toBeLessThan(2)
    await page.mouse.move(x - 880, y, { steps: 8 })
    const during = await state()
    expect(during).toMatchObject({ thread: 1300, main: 640, overflow: 120, scrollLeft: 120, navCollapsed: true, hscroll: false })
    expect(Math.abs(during.handle - (x - 880))).toBeLessThan(2)
    await page.mouse.up()

    // Otherwise the sideways scroll is the reader's: back to the start; reading up and down does not move it.
    await page.locator('.conversation').evaluate((element) => { element.scrollLeft = 0 })
    await page.mouse.move(700, 400)
    await page.mouse.wheel(0, 200)
    expect((await state()).scrollLeft).toBe(0)
    // A sideways wheel or trackpad over the messages scrolls the conversation (the message list keeps only its vertical scroll).
    await page.mouse.wheel(40, 0)
    await expect.poll(async () => (await state()).scrollLeft).toBeGreaterThan(0)
    // Shift and the wheel do the same. WebKit leaves that conversion to the system (macOS turns it into a sideways scroll
    // before the page sees it), so a synthesized Shift+wheel says nothing there.
    if (browserName !== 'webkit') {
      await page.locator('.conversation').evaluate((element) => { element.scrollLeft = 0 })
      await page.keyboard.down('Shift')
      await page.mouse.wheel(0, 40)
      await page.keyboard.up('Shift')
      await expect.poll(async () => (await state()).scrollLeft).toBeGreaterThan(0)
    }

    // The handle takes focus; Left and Right move it and keep it in view.
    await page.locator('.thread-column .resize-handle').focus()
    await page.keyboard.press('ArrowRight')
    await expect.poll(async () => (await state()).thread).toBe(1284)
    expect(await state()).toMatchObject({ overflow: 104, scrollLeft: 104 })
    await page.keyboard.press('Shift+ArrowLeft')
    await expect.poll(async () => (await state()).thread).toBe(1348)
    expect(await page.evaluate(() => localStorage.getItem('penfii.threadWidth'))).toBe('1348')

    // Nothing typed was lost on the way.
    await expect(main).toContainText('draft beside the thread')
    await page.evaluate(() => localStorage.setItem('penfii.threadWidth', '420'))
  })
})

test.describe('a thread opens without a sideways scroll on a narrow desktop', () => {
  test('until a width is set the thread opens at 420 or what leaves the main column 640, never under 280, and a set width is kept', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `qa-fit-${randomUUID().slice(0, 5)}`)
    const body = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'A thread on a narrow desktop. '.repeat(8) }] }] }
    const sent = await request.post(`/v1/conversations/${channel.conversationId}/messages`, {
      headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() },
      data: { bodySchemaVersion: 1, body, threadRootId: null, replyToId: null },
    })
    expect(sent.status()).toBe(201)
    const root = (await sent.json()) as { id: string }
    await logIn(page, accountA.email)
    await page.evaluate(() => { localStorage.removeItem('penfii.threadWidth'); localStorage.removeItem('penfii.contextNavWidth') })
    const state = () => page.evaluate(() => {
      const conversation = document.querySelector('.conversation')!
      const edge = document.querySelector('.nav-edge')?.getBoundingClientRect()
      const handle = document.querySelector('.thread-column .resize-handle')!.getBoundingClientRect()
      return {
        thread: Math.round(document.querySelector('.thread-column')!.getBoundingClientRect().width),
        main: Math.round(document.querySelector('.conversation > .column:not(.thread-column)')!.getBoundingClientRect().width),
        overflow: conversation.scrollWidth - conversation.clientWidth,
        // The "Show sidebar" tab has its own strip: nothing of the conversation runs under it.
        tabClear: !edge || edge.right <= conversation.getBoundingClientRect().left + 0.5,
        handleShown: handle.left >= 0 && handle.right <= innerWidth,
        stored: localStorage.getItem('penfii.threadWidth'),
      }
    })
    const open = async (width: number) => {
      await page.setViewportSize({ width, height: 800 })
      await page.goto(`/w/${workspaceId}/c/${channel.conversationId}/t/${root.id}`)
      await expect(page.locator('.thread-column .composer')).toBeVisible()
      return state()
    }
    // Rail 76, the nav's tab strip 24, main 640: 1024 leaves 284, 1100 leaves 360, from 1180 the thread gets its 420.
    for (const [width, thread] of [[1024, 284], [1100, 360], [1180, 420], [1280, 420], [1920, 420]]) {
      expect(await open(width), `at ${width}`).toMatchObject({ thread, overflow: 0, tabClear: true, handleShown: true, stored: null })
    }
    // 961 has room for only 245: the thread keeps 280 and the main column scrolls the rest, as for a wider thread.
    expect(await open(961)).toMatchObject({ thread: 280, main: 640, overflow: 59, tabClear: true, handleShown: true, stored: null })
    // A width the person set is theirs: kept as it is, with the sideways scroll.
    await page.evaluate(() => localStorage.setItem('penfii.threadWidth', '440'))
    expect(await open(1024)).toMatchObject({ thread: 440, main: 640, overflow: 156, tabClear: true, handleShown: true, stored: '440' })
    // A second press on the handle goes back to the default: the stored width is forgotten and the thread fits again.
    const handle = (await page.locator('.thread-column .resize-handle').boundingBox())!
    // Two presses 100ms apart (a synthesized double click can stamp both with the same time, which reads as one press).
    await page.mouse.click(handle.x + handle.width / 2, handle.y + 200)
    await page.waitForTimeout(100)
    await page.mouse.click(handle.x + handle.width / 2, handle.y + 200)
    await expect.poll(async () => (await state()).thread).toBe(284)
    expect(await state()).toMatchObject({ overflow: 0, stored: null })
  })
})
