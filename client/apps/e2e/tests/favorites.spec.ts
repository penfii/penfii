import { randomUUID } from 'node:crypto'
import { request as newRequest, type APIRequestContext, type Locator, type Page } from '@playwright/test'
import { expect, test } from '../support/test'
import { accountA, accountB } from '../support/accounts'
import { Api, apiAs, logIn } from '../support/api'

// FEA-199 to FEA-209 in the real app: every kind of Favorite is starred where it lives, found again in its own
// navigation and in Saved, opened from there, and removed; the order and folders of Saved hold across kinds; and
// nobody sees or saves what they cannot read.

const HEADERS = { 'x-penfii-client': '1' }
const BASE = process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080'

async function call<T = unknown>(request: APIRequestContext, method: string, path: string, data?: unknown, ok = true): Promise<T & { status?: number }> {
  const response = await request.fetch(path, { method, headers: { ...HEADERS, 'idempotency-key': randomUUID() }, data })
  if (ok) expect(response.status(), `${method} ${path}: ${await response.text()}`).toBeLessThan(300)
  const text = await response.text()
  const body = (text ? JSON.parse(text) : {}) as T & { status?: number }
  return ok ? body : { ...body, status: response.status() }
}

/** A request context signed in as `email`, for preparing data and reading back what the server keeps. */
async function as(email: string): Promise<{ request: APIRequestContext; id: string; name: string; workspaceId: string; dispose: () => Promise<void> }> {
  const request = await newRequest.newContext({ baseURL: BASE })
  const api = await apiAs(request, email)
  const me = await api.me()
  return { request, id: me.id, name: me.displayName, workspaceId: await api.workspaceId(), dispose: () => request.dispose() }
}

const star = (scope: Page | Locator, name: string | RegExp) => scope.getByRole('button', { name: typeof name === 'string' ? `Add ${name} to Favorites` : name })
const starred = (scope: Page | Locator, name: string) => scope.getByRole('button', { name: `Remove ${name} from Favorites` })
const page1 = (text: string) => ({ schema: 'penfii.doc', version: 1, blocks: [{ id: randomUUID(), type: 'paragraph', props: {}, content: [{ type: 'text', text, styles: {} }], children: [] }] })

async function favorites(request: APIRequestContext, workspaceId: string): Promise<Array<{ kind: string; targetId: string; folderId?: string | null; title: string | null }>> {
  return call(request, 'GET', `/v1/workspaces/${workspaceId}/favorites`)
}

async function clearFavorites(request: APIRequestContext, workspaceId: string) {
  for (const item of await favorites(request, workspaceId)) await call(request, 'DELETE', `/v1/workspaces/${workspaceId}/favorites/${item.kind}/${item.targetId}`)
  const folders = await call<{ items: Array<{ id: string }> }>(request, 'GET', `/v1/workspaces/${workspaceId}/favorite-folders`)
  for (const folder of folders.items) await call(request, 'DELETE', `/v1/favorite-folders/${folder.id}`)
}

test.describe.configure({ mode: 'serial' })

test.describe('Favorites of every kind (FEA-199)', () => {
  test('a Project, a Ticket on the board and in the table, and saved Views', async ({ page }) => {
    const a = await as(accountA.email)
    const ws = a.workspaceId
    await clearFavorites(a.request, ws)
    const tag = randomUUID().slice(0, 6)
    const project = await new Api(a.request).createProject(ws, `QA fav ${tag}`)
    const ticket = await call<{ id: string; key: string }>(a.request, 'POST', `/v1/projects/${project.id}/tickets`, { title: `Fav ticket ${tag}` })
    const first = await call<{ id: string }>(a.request, 'POST', `/v1/projects/${project.id}/views`, { name: `View one ${tag}`, scope: 'PERSONAL' })
    const second = await call<{ id: string }>(a.request, 'POST', `/v1/projects/${project.id}/views`, { name: `View two ${tag}`, scope: 'PROJECT' })
    await logIn(page, accountA.email)

    // The Project, from its card in My projects: the card does not open.
    await page.goto(`/w/${ws}/projects`)
    await star(page, `QA fav ${tag}`).click()
    await expect(starred(page, `QA fav ${tag}`)).toHaveAttribute('aria-pressed', 'true')
    await expect(page).toHaveURL(new RegExp(`/w/${ws}/projects$`))
    await expect(page.getByText('Added to Favorites')).toBeVisible()
    await expect(page.getByRole('region', { name: 'Favorites' }).getByRole('link', { name: `QA fav ${tag}` })).toBeVisible()

    // The Ticket from its board card: the panel does not open and nothing moves.
    await page.goto(`/w/${ws}/projects/${project.id}/work`)
    const card = page.locator(`[data-ticket-id="${ticket.id}"]`)
    await card.hover()
    await star(card, ticket.key).click()
    await expect(starred(card, ticket.key)).toHaveAttribute('aria-pressed', 'true')
    await expect(page).not.toHaveURL(/panel=ticket/)
    // The same state in the table (a Project has Board and Table), from the one shared list.
    await page.getByRole('button', { name: 'Table', exact: true }).click()
    const row = page.locator(`tr[data-ticket-id="${ticket.id}"]`)
    await row.hover()
    await starred(row, ticket.key).click()
    await expect(star(row, ticket.key)).toHaveAttribute('aria-pressed', 'false')
    await star(row, ticket.key).click()
    await expect(starred(row, ticket.key)).toHaveAttribute('aria-pressed', 'true')
    await expect(page).not.toHaveURL(/panel=ticket/)

    // Saved Views: the shared one comes first by itself; starring the personal one puts it first, and does not open it.
    const tabs = page.getByRole('group', { name: /Views/ })
    const position = async (name: string) => (await tabs.locator('.view-tab').allTextContents()).findIndex((text) => text.startsWith(name))
    expect(await position(`View two ${tag}`)).toBeLessThan(await position(`View one ${tag}`))
    await tabs.locator('.view-tab-slot', { hasText: `View one ${tag}` }).hover()
    await star(tabs, `View one ${tag}`).click()
    await expect(starred(tabs, `View one ${tag}`)).toHaveAttribute('aria-pressed', 'true')
    await expect(page).not.toHaveURL(new RegExp(`v=${first.id}`))
    await expect.poll(() => position(`View one ${tag}`)).toBe(1)
    expect(await position(`View two ${tag}`)).toBe(2)

    // Saved: every one with its own link, in the order they were starred; reopen the Ticket from there.
    await page.goto(`/w/${ws}/saved`)
    const links = page.locator('.favorites .saved-open')
    await expect(links).toHaveCount(3)
    await expect(links.nth(0)).toHaveAttribute('href', `/w/${ws}/projects/${project.id}`)
    await expect(links.nth(1)).toHaveAttribute('href', `/w/${ws}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    await expect(links.nth(2)).toHaveAttribute('href', `/w/${ws}/projects/${project.id}/work?v=${first.id}`)
    await links.nth(2).click()
    await expect(page).toHaveURL(new RegExp(`v=${first.id}`))
    await expect(tabs.getByRole("button", { name: `View one ${tag}`, exact: true, pressed: true })).toBeVisible()
    await page.goBack()
    await links.nth(1).click()
    await expect(page).toHaveURL(new RegExp(`panel=ticket:${ticket.id}`))
    await expect(page.locator('.ticket-panel').getByText(`Fav ticket ${tag}`).first()).toBeVisible()

    // Remove from Saved, undo once, remove again; a reload shows what the server kept.
    await page.goto(`/w/${ws}/saved`)
    await page.getByRole('button', { name: `Remove QA fav ${tag} from Favorites` }).click()
    await page.getByRole('button', { name: 'Undo' }).click()
    await expect(page.locator('.favorites .saved-open')).toHaveCount(3)
    await page.getByRole('button', { name: `Remove ${ticket.key} Fav ticket ${tag} from Favorites` }).click()
    await expect(page.locator('.favorites .saved-open')).toHaveCount(2)
    await page.reload()
    await expect(page.locator('.favorites .saved-open')).toHaveCount(2)
    expect((await favorites(a.request, ws)).map((item) => item.targetId)).toEqual([project.id, first.id])
    expect((await favorites(a.request, ws)).some((item) => item.targetId === second.id)).toBe(false)
    await clearFavorites(a.request, ws)
    // Projects have no delete; the Views made here go.
    const views = await call<Array<{ id: string; version: number }>>(a.request, 'GET', `/v1/projects/${project.id}/views`)
    for (const view of views) await call(a.request, 'DELETE', `/v1/views/${view.id}?expectedVersion=${view.version}`)
    await a.dispose()
  })

  test('channels, a Project channel and a DM, in Chat Favorites', async ({ page }) => {
    const a = await as(accountA.email)
    const b = await as(accountB.email)
    const ws = a.workspaceId
    await clearFavorites(a.request, ws)
    const tag = randomUUID().slice(0, 6)
    const channel = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/channels`, { name: `qa-fav-${tag}`, visibility: 'PUBLIC' })
    const project = await new Api(a.request).createProject(ws, `QA fav chat ${tag}`)
    const inProject = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/channels`, { name: `qa-fav-p-${tag}`, visibility: 'PRIVATE', projectId: project.id })
    const dm = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/dms`, { userIds: [a.id, b.id] })
    await logIn(page, accountA.email)

    // A channel, from its header.
    await page.goto(`/w/${ws}/c/${channel.id}`)
    await page.locator('.context-bar').getByRole('button', { name: 'Add to Favorites' }).click()
    await expect(page.locator('.context-bar').getByRole('button', { name: 'Remove from Favorites' })).toHaveAttribute('aria-pressed', 'true')
    const shortcuts = page.locator('.favorite-conversations')
    await expect(shortcuts.getByRole('link', { name: new RegExp(`qa-fav-${tag}`) })).toHaveAttribute('href', `/w/${ws}/c/${channel.id}`)
    // The DM, from its header: named by the other person.
    await page.goto(`/w/${ws}/c/${dm.id}`)
    await page.locator('.context-bar').getByRole('button', { name: 'Add to Favorites' }).click()
    await expect(shortcuts.getByRole('link', { name: new RegExp(b.name) })).toHaveAttribute('href', `/w/${ws}/c/${dm.id}`)
    // The Project channel, from its row in the Project's navigation; it opens in Chat from Favorites.
    await page.goto(`/w/${ws}/projects/${project.id}/chat`)
    const projectRow = page.locator('.nav-branch li', { hasText: `qa-fav-p-${tag}` })
    await projectRow.hover()
    await star(projectRow, `# qa-fav-p-${tag}`).click()
    await expect(starred(projectRow, `# qa-fav-p-${tag}`)).toHaveAttribute('aria-pressed', 'true')
    await page.goto(`/w/${ws}/chat`)
    await expect(shortcuts.getByRole('link')).toHaveCount(3)
    await expect(shortcuts.getByRole('link', { name: new RegExp(`qa-fav-p-${tag}`) })).toHaveAttribute('href', `/w/${ws}/c/${inProject.id}`)
    // Still in its own group: the channel list keeps it.
    await expect.poll(() => page.getByRole("link", { name: new RegExp(`qa-fav-${tag}`) }).count()).toBeGreaterThanOrEqual(2)
    // Saved links: the Project channel opens in its Project, the DM in Chat.
    await page.goto(`/w/${ws}/saved`)
    await expect(page.locator('.favorites .saved-open', { hasText: `qa-fav-p-${tag}` })).toHaveAttribute('href', `/w/${ws}/projects/${project.id}/chat/${inProject.id}`)
    await expect(page.locator('.favorites .saved-open', { hasText: b.name })).toHaveAttribute('href', `/w/${ws}/c/${dm.id}`)

    // Remove from the row menu in Favorites; a reload agrees.
    await page.goto(`/w/${ws}/chat`)
    const dmRow = shortcuts.locator('li', { hasText: b.name })
    await dmRow.getByRole('button', { name: 'Conversation menu' }).click()
    await page.getByRole('menuitemcheckbox', { name: /Remove from Favorites/ }).click()
    await expect(shortcuts.getByRole('link')).toHaveCount(2)
    await page.reload()
    await expect(shortcuts.getByRole('link')).toHaveCount(2)

    // B is not in the private Project channel: it cannot be saved, and A's shortcuts are A's.
    const refused = await call(b.request, 'PUT', `/v1/workspaces/${ws}/favorites`, { kind: 'CHANNEL', targetId: inProject.id }, false)
    expect(refused.status).toBe(404)
    const wrongKind = await call(a.request, 'PUT', `/v1/workspaces/${ws}/favorites`, { kind: 'CHANNEL', targetId: dm.id }, false)
    expect(wrongKind.status).toBe(404)
    expect((await favorites(b.request, ws)).some((item) => item.targetId === channel.id)).toBe(false)
    await clearFavorites(a.request, ws)
    await a.dispose()
    await b.dispose()
  })

  test('Docs in the global tree and in their Project, read-only and restricted', async ({ page }) => {
    const a = await as(accountA.email)
    const b = await as(accountB.email)
    const ws = a.workspaceId
    await clearFavorites(a.request, ws)
    await clearFavorites(b.request, ws)
    const tag = randomUUID().slice(0, 6)
    const project = await new Api(a.request).createProject(ws, `QA fav docs ${tag}`)
    const open = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/docs`, { title: `Fav open ${tag}`, visibility: 'WORKSPACE', content: page1('open') })
    const inProject = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/docs`, { title: `Fav project ${tag}`, visibility: 'WORKSPACE', projectId: project.id, content: page1('project') })
    const shared = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/docs`, { title: `Fav shared ${tag}`, visibility: 'RESTRICTED', participants: [{ userId: b.id, role: 'VIEWER' }], content: page1('shared') })
    const closed = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/docs`, { title: `Fav closed ${tag}`, visibility: 'RESTRICTED', content: page1('closed') })
    await logIn(page, accountA.email)

    // From the page itself, then the tree's Favorites lists it.
    await page.goto(`/w/${ws}/docs?doc=${open.id}`)
    await page.locator('.doc-actions').getByRole('button', { name: 'Add to Favorites' }).click()
    const section = page.locator('.doc-favorites')
    await expect(section.getByRole('button', { name: `Fav open ${tag}`, exact: true })).toBeVisible()
    // From a tree row in the Project's docs; the Project's Favorites hold only that Project's page.
    await page.goto(`/w/${ws}/projects/${project.id}/docs`)
    const treeRow = page.locator('.doc-tree-row', { hasText: `Fav project ${tag}` }).first()
    await treeRow.hover()
    await star(treeRow, `Fav project ${tag}`).click()
    await expect(section.first().getByRole('button', { name: `Fav project ${tag}`, exact: true })).toBeVisible()
    await expect(page.locator('.doc-favorites', { hasText: `Fav open ${tag}` })).toHaveCount(0)
    await section.first().getByRole('button', { name: `Fav project ${tag}`, exact: true }).click()
    await expect(page).toHaveURL(new RegExp(`doc=${inProject.id}`))
    // Saved opens the exact page; a rename shows there.
    await call(a.request, 'PATCH', `/v1/docs/${open.id}`, { title: `Fav open renamed ${tag}` })
    await page.goto(`/w/${ws}/saved`)
    const link = page.locator('.favorites .saved-open', { hasText: `Fav open renamed ${tag}` })
    await expect(link).toHaveAttribute('href', `/w/${ws}/docs?doc=${open.id}`)
    await link.click()
    await expect(page.getByRole('heading', { name: `Fav open renamed ${tag}` })).toBeVisible()

    // B may star the page shared with B (read-only) but not the closed one; losing the share hides it.
    expect((await call<{ title: string }>(b.request, 'PUT', `/v1/workspaces/${ws}/favorites`, { kind: 'DOC', targetId: shared.id })).title).toBe(`Fav shared ${tag}`)
    expect((await call(b.request, 'PUT', `/v1/workspaces/${ws}/favorites`, { kind: 'DOC', targetId: closed.id }, false)).status).toBe(404)
    await call(a.request, 'PUT', `/v1/docs/${shared.id}/participants`, { participants: [{ userId: a.id, role: 'MANAGER' }] })
    expect((await favorites(b.request, ws)).some((item) => item.targetId === shared.id)).toBe(false)
    // A deleted page goes from the list, with no title left behind.
    await call(a.request, 'DELETE', `/v1/docs/${open.id}`)
    await page.goto(`/w/${ws}/saved`)
    await expect(page.locator('.favorites .saved-open')).toHaveCount(1)
    await expect(page.getByText(`Fav open renamed ${tag}`)).toHaveCount(0)
    // The hidden ones are kept by the server and not listed: remove them by name, then the test's pages.
    await clearFavorites(a.request, ws)
    await clearFavorites(b.request, ws)
    await call(a.request, 'DELETE', `/v1/workspaces/${ws}/favorites/DOC/${open.id}`)
    await call(b.request, 'DELETE', `/v1/workspaces/${ws}/favorites/DOC/${shared.id}`)
    for (const id of [inProject.id, shared.id, closed.id]) await call(a.request, 'DELETE', `/v1/docs/${id}`)
    await a.dispose()
    await b.dispose()
  })

  test('a Team from the directory, found in the Teams navigation and Saved; Home and search lead to Favorites', async ({ page }) => {
    const a = await as(accountA.email)
    const ws = a.workspaceId
    await clearFavorites(a.request, ws)
    const tag = randomUUID().slice(0, 6)
    const team = await call<{ id: string; status?: number }>(a.request, 'POST', `/v1/workspaces/${ws}/teams`, { name: `QA fav team ${tag}`, members: [{ userId: a.id, role: 'MANAGER' }] }, false)
    test.skip(team.status !== undefined && team.status >= 300, 'account A may not create a Team here')
    const project = await new Api(a.request).createProject(ws, `QA fav search ${tag}`)
    const ticket = await call<{ id: string; key: string }>(a.request, 'POST', `/v1/projects/${project.id}/tickets`, { title: `Searchable fav ${tag}` })
    await logIn(page, accountA.email)

    await page.goto(`/w/${ws}/directory/teams/${team.id}`)
    await star(page.locator('.page-intro'), `QA fav team ${tag}`).click()
    const nav = page.getByRole('region', { name: 'Favorites' })
    await expect(nav.getByRole('link', { name: new RegExp(`QA fav team ${tag}`) })).toHaveAttribute('href', `/w/${ws}/directory/teams/${team.id}`)
    await page.goto(`/w/${ws}/directory/teams`)
    await expect(starred(page.locator('.all-team-row', { hasText: `QA fav team ${tag}` }), `QA fav team ${tag}`)).toHaveAttribute('aria-pressed', 'true')

    // Home has a way to Saved; the Team there still leads to its work.
    await page.goto(`/w/${ws}`)
    await page.getByRole('link', { name: 'Open Saved' }).click()
    await expect(page).toHaveURL(new RegExp(`/w/${ws}/saved`))
    await expect(page.locator('.favorites .saved-open', { hasText: `QA fav team ${tag}` })).toHaveAttribute('href', `/w/${ws}/teams/${team.id}`)

    // Search: the star beside a Ticket result, which does not open the result. Indexing is asynchronous, so wait
    // until the search finds the Ticket before opening the page.
    await expect.poll(async () => {
      const found = await call<{ results?: Array<{ id: string }> }>(a.request, 'GET', `/v1/workspaces/${ws}/search?q=${encodeURIComponent(`Searchable fav ${tag}`)}&kind=ticket`)
      return (found.results ?? []).some((item) => item.id === ticket.id)
    }, { timeout: 60_000 }).toBe(true)
    await page.goto(`/w/${ws}/search?q=${encodeURIComponent(`Searchable fav ${tag}`)}`)
    const hit = page.locator('.search-hit', { hasText: `Searchable fav ${tag}` })
    await expect(hit).toBeVisible({ timeout: 20_000 })
    await star(hit, /Add .* to Favorites/).click()
    await expect(hit.getByRole('button', { name: /Remove .* from Favorites/ })).toHaveAttribute('aria-pressed', 'true')
    await expect(page).toHaveURL(/\/search/)
    expect((await favorites(a.request, ws)).map((item) => item.targetId)).toEqual([team.id, ticket.id])
    await clearFavorites(a.request, ws)
    await call(a.request, 'POST', `/v1/workspaces/${ws}/teams/${team.id}/archive`)
    await a.dispose()
  })

  test('Saved keeps the order of every group when one folder is reordered, and a narrow screen still works', async ({ page, allowProblem }) => {
    allowProblem(/503 PUT .*\/favorites\/order|Failed to load resource: the server responded with a status of 503/)
    const a = await as(accountA.email)
    const ws = a.workspaceId
    await clearFavorites(a.request, ws)
    const tag = randomUUID().slice(0, 6)
    const ids: string[] = []
    for (const name of ['one', 'two', 'three', 'four']) {
      const project = await new Api(a.request).createProject(ws, `QA order ${name} ${tag}`)
      await call(a.request, 'PUT', `/v1/workspaces/${ws}/favorites`, { kind: 'PROJECT', targetId: project.id })
      ids.push(project.id)
    }
    const doc = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/docs`, { title: `QA order doc ${tag}`, visibility: 'WORKSPACE', content: page1('x') })
    await call(a.request, 'PUT', `/v1/workspaces/${ws}/favorites`, { kind: 'DOC', targetId: doc.id })
    const folder = await call<{ id: string }>(a.request, 'POST', `/v1/workspaces/${ws}/favorite-folders`, { name: `Daily ${tag}` })
    // Folder: two and four. Top level: one, three, the doc.
    for (const id of [ids[1], ids[3]]) await call(a.request, 'PUT', `/v1/workspaces/${ws}/favorites/PROJECT/${id}/folder`, { folderId: folder.id })
    await page.setViewportSize({ width: 390, height: 800 })
    await logIn(page, accountA.email)
    await page.goto(`/w/${ws}/saved`)
    const inFolder = page.getByRole('region', { name: `Daily ${tag}` })
    await inFolder.getByRole('button', { name: `Move QA order four ${tag} up` }).click()
    await expect(inFolder.locator('.saved-open').first()).toContainText(`QA order four ${tag}`)
    const order = (await favorites(a.request, ws)).map((item) => item.targetId)
    // Only the folder's two swapped places; the top level kept its order and its places around them.
    expect(order).toEqual([ids[0], ids[3], ids[2], ids[1], doc.id])
    // A failed move says so and shows the saved order again.
    await page.route('**/v1/workspaces/*/favorites/order', (route) => route.fulfill({ status: 503, body: '{}' }))
    await page.getByRole('button', { name: `Move QA order doc ${tag} up` }).click()
    await expect(page.getByText('Could not change the order. Showing the saved order again.')).toBeVisible()
    await page.unroute('**/v1/workspaces/*/favorites/order')
    expect((await favorites(a.request, ws)).map((item) => item.targetId)).toEqual(order)
    // The page does not scroll sideways on a phone.
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true)
    await clearFavorites(a.request, ws)
    await call(a.request, 'DELETE', `/v1/docs/${doc.id}`)
    await a.dispose()
  })
})
