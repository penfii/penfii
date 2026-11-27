import { randomUUID } from 'node:crypto'
import { expect, test } from '../support/test'
import { accountA, PASSWORD } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// FEA-178 to FEA-187 in the real app: the login conveniences (FEA-180 to FEA-182) and the layout work (FEA-178, FEA-185 to FEA-187).

test.describe('FEA-180/181/182: signing in', () => {
  test('the fields are named for a password manager, and keep-signed-in gives a 30 day cookie that is not stored in the page', async ({ page, context }) => {
    await page.goto('/login')
    const email = page.getByLabel('Email', { exact: true })
    const password = page.getByLabel('Password', { exact: true })
    await expect(email).toHaveAttribute('name', 'email')
    await expect(email).toHaveAttribute('autocomplete', 'username')
    await expect(password).toHaveAttribute('name', 'password')
    await expect(password).toHaveAttribute('autocomplete', 'current-password')
    const keep = page.getByRole('checkbox', { name: /Keep me signed in/ })
    const remember = page.getByRole('checkbox', { name: 'Remember my ID' })
    await expect(keep).not.toBeChecked()
    await expect(remember).not.toBeChecked()
    await email.fill(accountA.email)
    await password.fill(PASSWORD)
    await keep.check()
    await remember.check()
    await page.getByRole('button', { name: /^Log in/ }).click()
    await expect(page).not.toHaveURL(/\/login/)
    const cookie = (await context.cookies()).find((item) => item.name === 'penfii_session')!
    const days = (cookie.expires - Date.now() / 1000) / 86_400
    expect(days).toBeGreaterThan(29)
    expect(days).toBeLessThanOrEqual(30)
    expect(cookie.httpOnly).toBe(true)
    // Nothing secret is kept by the page.
    const stored = await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))
    expect(stored).not.toContain(cookie.value)
    expect(stored).not.toContain(PASSWORD)
    expect(await page.evaluate(() => localStorage.getItem('penfii.login.email'))).toBe(accountA.email)
    expect(await page.evaluate(() => localStorage.getItem('penfii.login.method'))).toBe('password')
    // After signing out the sign-in page restores the address, both choices and the last method; unticking forgets the address.
    await page.getByRole('button', { name: accountA.name }).click()
    await page.getByRole('button', { name: 'Sign out' }).click()
    await expect(page).toHaveURL(/\/login/)
    await expect(page.getByLabel('Email', { exact: true })).toHaveValue(accountA.email)
    await expect(page.getByRole('checkbox', { name: 'Remember my ID' })).toBeChecked()
    await expect(page.getByRole('checkbox', { name: /Keep me signed in/ })).toBeChecked()
    await expect(page.getByRole('button', { name: /^Log in/ })).toContainText('Last used')
    await page.getByRole('checkbox', { name: 'Remember my ID' }).uncheck()
    expect(await page.evaluate(() => localStorage.getItem('penfii.login.email'))).toBeNull()
  })

  test('without the choice the cookie lasts the ordinary 12 hours', async ({ page, context }) => {
    await logIn(page, accountA.email)
    const cookie = (await context.cookies()).find((item) => item.name === 'penfii_session')!
    const hours = (cookie.expires - Date.now() / 1000) / 3600
    expect(hours).toBeGreaterThan(11)
    expect(hours).toBeLessThanOrEqual(12)
  })
})

test.describe('FEA-178: a document at full width', () => {
  test('the setting is saved at once for everyone, keeps an open draft, and a failed save puts the page back', async ({ page, request, allowProblem }) => {
    allowProblem(/500|Failed to load resource/)
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const doc = await api.createDoc(workspaceId, `QA wide ${randomUUID().slice(0, 5)}`)
    await page.setViewportSize({ width: 1600, height: 900 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/docs?doc=${doc.id}`)
    const article = page.locator('.doc-page')
    await expect(article).not.toHaveClass(/is-wide/)
    // The space left of the title: 6% of the column (at least 32px) by default, 32px at full width.
    const side = () => page.locator('.doc-column').evaluate((element) => element.querySelector('.doc-head')!.getBoundingClientRect().left - element.getBoundingClientRect().left)
    const narrow = await side()
    await page.getByRole('button', { name: 'More actions' }).click()
    const toggle = page.getByRole('menuitemcheckbox', { name: 'Full width' })
    await expect(toggle).toHaveAttribute('aria-checked', 'false')
    await toggle.click()
    await expect(article).toHaveClass(/is-wide/)
    expect(await side()).toBeLessThan(narrow)
    expect(Math.round(await side())).toBe(32)
    // Saved without Save: another read of the page says so, and the revision did not move.
    await expect.poll(async () => (await request.get(`/v1/docs/${doc.id}`)).json().then((page) => [page.fullWidth, page.revisionNo])).toEqual([true, doc.revisionNo])
    // Both panels stay, and the page never scrolls sideways.
    await expect(page.locator('.doc-aside')).toBeVisible()
    expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false)
    await page.reload()
    await expect(page.locator('.doc-page')).toHaveClass(/is-wide/)

    // While a draft is open the setting still works and the draft survives and saves against its own revision.
    await page.getByRole('button', { name: 'Edit', exact: true }).click()
    const editor = page.locator('.bn-editor[contenteditable=true]').first()
    await editor.click()
    await page.keyboard.type('draft-text-fea178')
    await page.getByRole('button', { name: 'More actions' }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Full width' }).click()
    await expect(page.locator('.doc-page')).not.toHaveClass(/is-wide/)
    await expect(editor).toContainText('draft-text-fea178')
    await page.getByRole('button', { name: 'Save', exact: true }).click()
    await expect.poll(async () => (await request.get(`/v1/docs/${doc.id}`)).json().then((page) => [page.fullWidth, JSON.stringify(page.content).includes('draft-text-fea178')])).toEqual([false, true])

    // A refused save: the page returns to what is stored and says so.
    await page.route(new RegExp(`/v1/docs/${doc.id}$`), (route) => (route.request().method() === 'PATCH' ? route.fulfill({ status: 500, contentType: 'application/json', body: '{"code":"internal_error","message":"x"}' }) : route.continue()))
    await page.getByRole('button', { name: 'More actions' }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Full width' }).click()
    await expect(page.getByText('The layout could not be saved. The page shows what is stored.').first()).toBeVisible()
    await expect(page.locator('.doc-page')).not.toHaveClass(/is-wide/)
  })
})

test.describe('FEA-185: the project name in the header', () => {
  test('it is one line, whole while there is room, cut with the full name on hover only when there is not', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const short = await api.createProject(workspaceId, `feature ${randomUUID().slice(0, 3)}`)
    const long = await api.createProject(workspaceId, `A very long project name in English that goes on and on for the header test ${randomUUID().slice(0, 4)}`)
    await page.setViewportSize({ width: 1600, height: 900 })
    await logIn(page, accountA.email)
    const measure = () => page.evaluate(() => {
      const label = document.querySelector('.project-title-name .pop-label, .project-title-name')!.querySelector('[data-fit]') as HTMLElement
      const left = document.querySelector('.project-title-left')!.getBoundingClientRect()
      const right = document.querySelector('.project-title-right')!.getBoundingClientRect()
      return { cut: label.scrollWidth > label.clientWidth + 1, rowHeight: Math.round(document.querySelector('.project-title-row')!.getBoundingClientRect().height), overlap: left.right > right.left + 1, hscroll: document.documentElement.scrollWidth > innerWidth, lines: Math.round(label.getBoundingClientRect().height / parseFloat(getComputedStyle(label).lineHeight)) }
    })
    await page.goto(`/w/${workspaceId}/projects/${short.id}`)
    await expect(page.locator('.project-title-name')).toBeVisible()
    const whole = await measure()
    expect(whole).toMatchObject({ cut: false, overlap: false, hscroll: false })
    expect(whole.lines).toBe(1)
    await page.goto(`/w/${workspaceId}/projects/${long.id}`)
    await expect(page.locator('.project-title-name')).toBeVisible()
    const wide = await measure()
    expect(wide.rowHeight).toBe(whole.rowHeight)
    expect(wide.lines).toBe(1)
    for (const width of [1000, 700]) {
      await page.setViewportSize({ width, height: 900 })
      const narrow = await measure()
      expect(narrow).toMatchObject({ cut: true, overlap: false, hscroll: false, rowHeight: whole.rowHeight })
      expect(narrow.lines).toBe(1)
    }
    await page.locator('.project-title-name').hover()
    await expect(page.locator('.project-title-name')).toHaveAttribute('data-full', /A very long project name/)
  })
})

test.describe('FEA-186: the timeline calendar', () => {
  test('long titles end in an ellipsis inside their cell or bar, and a narrow window scrolls the calendar alone', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA calendar ${randomUUID().slice(0, 4)}`)
    const day = [{ from: '09:00', to: '18:00' }]
    await request.put(`/v1/projects/${project.id}/calendar`, { headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() }, data: { expectedVersion: 0, timeZone: 'UTC', definition: { version: 1, week: { mon: day, tue: day, wed: day, thu: day, fri: day, sat: [], sun: [] }, holidays: [] } } })
    const today = new Date().toISOString().slice(0, 10)
    const long = 'VeryLongTitleWithoutAnySpacesToTestTheCalendarCell'.repeat(2)
    const create = async (data: Record<string, unknown>) => {
      const response = await request.post(`/v1/projects/${project.id}/tickets`, { headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID() }, data })
      expect(response.status()).toBe(201)
    }
    await create({ title: long, dueOn: today })
    await create({ title: `${long} plan`, schedule: { plannedStartAt: `${today}T00:00:00Z`, durationMinutes: 540 * 3 } })
    await page.setViewportSize({ width: 1600, height: 900 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/timeline?mode=calendar&plan=1`)
    await expect(page.locator('.tlcalendar-table')).toBeVisible()
    const cut = await page.evaluate(() => {
      const outside = (element: Element, box: Element) => element.getBoundingClientRect().right > box.getBoundingClientRect().right + 1
      const due = [...document.querySelectorAll('.cal-due')]
      const plans = [...document.querySelectorAll('button.cal-plan')]
      return { dueCut: due.every((element) => element.scrollWidth > element.clientWidth && !outside(element, element.closest('td')!)), plans: plans.length, planInTable: plans.every((element) => !outside(element, document.querySelector('.tlcalendar-table')!)) }
    })
    expect(cut.dueCut).toBe(true)
    expect(cut.plans).toBeGreaterThan(0)
    expect(cut.planInTable).toBe(true)
    // Wide: no scrolling box. Narrow: the table stops shrinking and the box around it scrolls, the page and the weekday row with it.
    const geometry = () => page.evaluate(() => {
      const box = document.querySelector('.tlcalendar-scroll') as HTMLElement
      const table = document.querySelector('.tlcalendar-table') as HTMLElement
      return { scrolls: box.scrollWidth > box.clientWidth, table: Math.round(table.getBoundingClientRect().width), hscroll: document.documentElement.scrollWidth > innerWidth }
    })
    expect((await geometry()).scrolls).toBe(false)
    await page.setViewportSize({ width: 800, height: 900 })
    const narrow = await geometry()
    expect(narrow).toMatchObject({ scrolls: true, hscroll: false })
    expect(narrow.table).toBeGreaterThanOrEqual(7 * 16 * 7 - 2)
    await page.locator('.tlcalendar-scroll').evaluate((element) => { element.scrollLeft = element.scrollWidth })
    await expect(page.getByRole('columnheader').last()).toBeInViewport()
    expect((await geometry()).hscroll).toBe(false)
  })
})

test.describe('FEA-187: the Work toolbar', () => {
  test('it stays on one line while the side panel widens, scrolls by itself, and opens its menus uncut', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA toolbar ${randomUUID().slice(0, 4)}`)
    const ticket = await api.createTicket(project.id, 'Toolbar ticket')
    await page.setViewportSize({ width: 1600, height: 900 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work`)
    const row = page.locator('.view-bar-row')
    await expect(row).toBeVisible()
    const state = () => page.evaluate(() => {
      const bar = document.querySelector('.view-bar-row') as HTMLElement
      const tops = [...bar.querySelectorAll('button, input, select')].map((element) => Math.round(element.getBoundingClientRect().top))
      return { oneLine: Math.max(...tops) - Math.min(...tops) < 20, scrolls: bar.scrollWidth > bar.clientWidth, height: Math.round(bar.getBoundingClientRect().height), board: Math.round(document.querySelector('.board-viewport')!.getBoundingClientRect().top), hscroll: document.documentElement.scrollWidth > innerWidth }
    })
    const wide = await state()
    expect(wide.oneLine).toBe(true)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket%3A${ticket.id}`)
    await expect(page.getByRole('button', { name: 'Display' })).toBeVisible()
    // Widen the ticket panel by dragging its handle.
    const handle = page.locator('[role=separator]').last()
    const box = (await handle.boundingBox())!
    await page.mouse.move(box.x + box.width / 2, box.y + 200)
    await page.mouse.down()
    for (const x of [1200, 900, 700]) await page.mouse.move(x, box.y + 200, { steps: 4 })
    await page.mouse.up()
    const narrow = await state()
    expect(narrow).toMatchObject({ oneLine: true, scrolls: true, hscroll: false, height: wide.height, board: wide.board })
    // Scrolling the bar does not move the page or the Board, and its last control opens a menu that is not cut off.
    await row.evaluate((element) => { element.scrollLeft = element.scrollWidth })
    await page.getByRole('button', { name: 'Display' }).click()
    const panel = page.locator('.pop-panel').first()
    await expect(panel).toBeVisible()
    const placed = await panel.evaluate((element) => {
      const rect = element.getBoundingClientRect()
      return { inside: rect.left >= 0 && rect.right <= innerWidth && rect.top >= 0 && rect.bottom <= innerHeight, text: element.scrollWidth <= element.clientWidth + 1 }
    })
    expect(placed).toEqual({ inside: true, text: true })
    await expect(panel.getByRole('checkbox', { name: /subtasks/ })).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.locator('.pop-panel')).toHaveCount(0)
    expect((await state()).board).toBe(wide.board)
  })
})

test.describe('Board: keyboard moves', () => {
  test('a card moved with Alt+Right into an empty column keeps the keyboard focus', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `QA focus ${randomUUID().slice(0, 4)}`)
    await api.createTicket(project.id, 'Focus ticket')
    await page.setViewportSize({ width: 1440, height: 900 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work`)
    await page.getByRole('button', { name: 'Board', exact: true }).click()
    const open = page.getByRole('button', { name: /^Open .*-001$/ })
    await open.focus()
    await page.keyboard.press('Alt+ArrowRight')
    // The card is now in the next column and is still the focused element.
    await expect(page.getByRole('region', { name: 'In Progress' }).getByText('Focus ticket')).toBeVisible()
    await expect.poll(() => page.evaluate(() => document.activeElement?.closest('[data-ticket-id]')?.textContent ?? '')).toContain('Focus ticket')
  })
})
