import { expect, test, type Page, type Response } from '@playwright/test'
import { Journal, people, projectSocketReady, Stage, together, userSocketReady, type Actor, type EngineName } from '../support/multiUser'

// Several people at once, each signed in in a browser context of their own, on different browser engines:
//   owner   -> Chromium   (workspace OWNER, makes and administers the Project)
//   member  -> Firefox    (MEMBER, Project editor, in the private channel)
//   member2 -> WebKit     (MEMBER, Project editor, not in the private channel)
//   outsider-> WebKit     (MEMBER, not in the Project; a second WebKit context)
//   admin   -> Chromium   (workspace ADMIN, not in the Project; a second Chromium context)
// It needs the five accounts (apps/e2e/seed-multi-user.sh) and all three engines, so it is opt-in:
// E2E_MULTI_USER=1. Actions that should happen at the same moment are started together and the test waits on what
// the screens and the network show, never on a fixed delay.

test.skip(!process.env.E2E_MULTI_USER, 'set E2E_MULTI_USER=1 (and make the accounts) to run the multi-user tests')
test.skip(({ browserName }) => browserName !== 'chromium', 'it launches its own engines; it runs once, under the chromium project')
test.describe.configure({ mode: 'serial' })
test.setTimeout(180_000)

const baseURL = process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080'
const journal = new Journal(process.env.E2E_MU_LOG ?? `test-results/multi-user/run-${new Date().toISOString().replace(/[:.]/g, '-')}.jsonl`)
const stage = new Stage(journal, baseURL)
const stamp = Date.now().toString(36)
// E2E_MU_ENGINES=owner=webkit,member=chromium,member2=firefox moves people to other engines (to tell an engine's
// problem from the application's).
const assignment = new Map((process.env.E2E_MU_ENGINES ?? '').split(',').filter(Boolean).map((pair) => pair.split('=') as [string, EngineName]))
const engineFor = (key: string, fallback: EngineName): EngineName => assignment.get(key) ?? fallback

let owner: Actor, member: Actor, member2: Actor, outsider: Actor, admin: Actor
let workspaceId = ''
let workspaceName = ''
let projectId = ''
const room = { name: `mu-room-${stamp}`, id: '' }
const secret = { name: `mu-secret-${stamp}`, id: '' }
const ids: Record<string, string> = {}

const w = (path = '') => `${baseURL}/w/${workspaceId}${path}`
const projectUrl = (path = '') => w(`/projects/${projectId}${path}`)
const sidebar = (page: Page) => page.getByRole('complementary').first()
const projectCard = (page: Page, name: string) => page.locator('.result-list a.result-hit').filter({ hasText: name })
const notFound = (page: Page) => page.locator('[data-state="not-found"]')
const apiResponse = (page: Page, method: string, path: RegExp) =>
  page.waitForResponse((response) => response.request().method() === method && path.test(new URL(response.url()).pathname))

test.beforeAll(async () => {
  test.setTimeout(180_000)
  journal.write('stage', 'start', `base ${baseURL}`)
  ;[owner, member, member2, outsider, admin] = await Promise.all([
    stage.enter(people.owner, engineFor('owner', 'chromium')),
    stage.enter(people.member, engineFor('member', 'firefox')),
    stage.enter(people.member2, engineFor('member2', 'webkit')),
    stage.enter(people.outsider, engineFor('outsider', 'webkit')),
    stage.enter(people.admin, engineFor('admin', 'chromium')),
  ])
  for (const actor of stage.actors) {
    const me = await actor.json<{ id: string }>('GET', '/v1/me')
    ids[actor.person.key] = me.id
    journal.write(actor.tag, 'account', `${actor.person.email} (${actor.person.role}) id ${me.id}`)
  }
  const workspaces = await owner.json<Array<{ id: string; name: string }>>('GET', '/v1/workspaces')
  workspaceId = workspaces[0].id
  workspaceName = workspaces[0].name
})

test.afterEach(async ({}, testInfo) => {
  const problems = stage.settle()
  journal.write('stage', 'test end', `${testInfo.title}: ${testInfo.status}; ${problems.length} unexpected browser problems`)
  expect(problems, 'browser problems (console, page errors, failed or refused requests, socket errors)').toEqual([])
})

test.afterAll(async () => {
  await stage.close()
})

test('sessions are separate: each context is its own person', async () => {
  const cookies = await Promise.all(stage.actors.map(async (actor) => (await actor.context.cookies()).map((cookie) => `${cookie.name}=${cookie.value}`).join(';')))
  expect(new Set(cookies).size, 'every context holds its own session cookie').toBe(stage.actors.length)
  for (const actor of stage.actors) {
    await expect(actor.page.getByRole('button', { name: actor.person.name, exact: true })).toBeVisible()
    for (const other of stage.actors.filter((item) => item !== actor)) {
      await expect(actor.page.getByRole('button', { name: other.person.name, exact: true })).toHaveCount(0)
    }
  }
})

test('a Project and its channels made by the owner show up only for the people in them', async () => {
  const projectName = `MU Board ${stamp}`
  await owner.act('create the Project in the New project dialog', async () => {
    await owner.page.goto(w('/projects'))
    await owner.page.getByRole('banner').getByRole('button', { name: '+ New project' }).click()
    const dialog = owner.page.getByRole('dialog', { name: 'New project' })
    await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill(projectName)
    await expect(dialog.getByRole('button', { name: 'Create project' })).toBeEnabled()
    await dialog.getByRole('button', { name: 'Create project' }).click()
    await expect(owner.page).toHaveURL(/\/projects\/[0-9a-f-]{36}$/)
    projectId = owner.page.url().split('/projects/')[1]
  })

  // Before anyone is let in, the others look for it at the same time. Project names are a workspace directory (by
  // design, since v0.1.0): the Project is listed as "Join needed" and its content is closed.
  await together(
    ...[member, member2].map((actor) => () => actor.act('look for the Project before being let in', async () => {
      // Since v0.3.0 the Projects page opens on "My projects"; a Project the person is not in is under All projects.
      await actor.page.goto(w('/projects?browse=all'))
      await expect(projectCard(actor.page, projectName)).toContainText('Join needed')
      await actor.page.goto(projectUrl('/work'))
      await expect(actor.page.getByText('You are not part of this project.')).toBeVisible()
    })),
  )

  await owner.act('give MU Member and MU Member Two the Editor role in Project settings', async () => {
    await owner.page.goto(projectUrl('/settings'))
    const permissions = owner.page.getByRole('region', { name: 'Permissions' })
    for (const person of [people.member, people.member2]) {
      await permissions.getByRole('combobox', { name: 'Role to give' }).selectOption({ label: 'Editor' })
      const saved = apiResponse(owner.page, 'PUT', /\/grants\//)
      await permissions.getByRole('listitem').getByRole('button', { name: person.name, exact: true }).click()
      expect((await saved).status()).toBe(204)
      await expect(permissions.getByRole('combobox', { name: `${person.name}: Role` })).toHaveValue('EDITOR')
    }
  })

  for (const [channel, visibility] of [[room, 'Project members'], [secret, 'Private']] as const) {
    await owner.act(`create the ${visibility} channel ${channel.name}`, async () => {
      await owner.page.goto(projectUrl())
      await sidebar(owner.page).getByRole('button', { name: 'Create channel' }).click()
      const dialog = owner.page.getByRole('dialog', { name: 'New channel' })
      await dialog.getByRole('textbox', { name: 'Channel name' }).fill(channel.name)
      await dialog.getByRole('combobox').selectOption({ label: visibility })
      await dialog.getByRole('button', { name: 'Create channel' }).click()
      await expect(owner.page.getByRole('heading', { name: `# ${channel.name}`, level: 1 })).toBeVisible()
      channel.id = owner.page.url().split('/chat/')[1]
    })
  }

  await owner.act('add MU Member to the private channel', async () => {
    await owner.page.goto(projectUrl(`/chat/${secret.id}?settings=1`))
    await owner.page.getByRole('dialog', { name: 'Channel settings' }).getByRole('button', { name: 'Add people' }).click()
    const dialog = owner.page.getByRole('dialog', { name: 'Add people to this channel' })
    const added = apiResponse(owner.page, 'PUT', /\/channels\/.*\/members\//)
    await dialog.getByRole('button', { name: people.member.name, exact: true }).click()
    expect((await added).status()).toBeLessThan(300)
    await expect(dialog.getByRole('list', { name: 'Members' })).toContainText(people.member.name)
    await dialog.getByRole('button', { name: 'Close' }).click()
  })

  // Everyone looks again at the same moment, after a navigation.
  await together(
    () => member.act('see the Project and both channels', async () => {
      await member.page.goto(w('/projects?browse=all'))
      await expect(projectCard(member.page, projectName)).toContainText('Taking part')
      await projectCard(member.page, projectName).click()
      await expect(member.page.getByRole('heading', { name: projectName, level: 1 })).toBeVisible()
      await sidebar(member.page).getByRole('button', { name: 'Expand Chat' }).click()
      await expect(sidebar(member.page).getByRole('link', { name: `# ${room.name}` })).toBeVisible()
      await expect(sidebar(member.page).getByRole('link', { name: `# ${secret.name}` })).toBeVisible()
    }),
    () => member2.act('see the Project and only the Project-members channel', async () => {
      await member2.page.goto(w('/projects?browse=all'))
      await expect(projectCard(member2.page, projectName)).toContainText('Taking part')
      await projectCard(member2.page, projectName).click()
      await expect(member2.page.getByRole('heading', { name: projectName, level: 1 })).toBeVisible()
      await sidebar(member2.page).getByRole('button', { name: 'Expand Chat' }).click()
      await expect(sidebar(member2.page).getByRole('link', { name: `# ${room.name}` })).toBeVisible()
      await expect(sidebar(member2.page).getByRole('link', { name: `# ${secret.name}` })).toHaveCount(0)
    }),
    ...[outsider, admin].map((actor) => () => actor.act('find the Project closed (not taking part)', async () => {
      // Since v0.3.0 the Projects page opens on "My projects"; a Project the person is not in is under All projects.
      await actor.page.goto(w('/projects?browse=all'))
      await expect(projectCard(actor.page, projectName)).toContainText('Join needed')
      await actor.page.goto(projectUrl('/chat'))
      await expect(actor.page.getByText('You are not part of this project.')).toBeVisible()
      await expect(sidebar(actor.page).getByRole('link', { name: `# ${room.name}` })).toHaveCount(0)
    })),
  )
})

test('people without access are refused, by the screens and by the API', async () => {
  for (const actor of [member2, outsider, admin]) actor.allow(/^40[34] /)
  await together(
    () => member2.act('open the private channel by its address', async () => {
      await member2.page.goto(projectUrl(`/chat/${secret.id}`))
      await expect(notFound(member2.page)).toBeVisible()
      await expect(member2.page.getByRole('textbox', { name: 'Write a message' })).toHaveCount(0)
    }),
    () => outsider.act('open the Project and its channel by address', async () => {
      await outsider.page.goto(projectUrl('/work'))
      await expect(outsider.page.getByText('You are not part of this project.')).toBeVisible()
      await outsider.page.goto(w(`/c/${room.id}`))
      await expect(notFound(outsider.page)).toBeVisible()
    }),
    () => admin.act('open the private channel by address', async () => {
      await admin.page.goto(w(`/c/${secret.id}`))
      await expect(notFound(admin.page)).toBeVisible()
    }),
  )
  const refused = async (actor: Actor, method: string, path: string, body?: unknown) => {
    const response = await actor.api(method, path, body)
    expect([403, 404], `${actor.tag} ${method} ${path} -> ${response.status()}`).toContain(response.status())
  }
  await refused(member2, 'GET', `/v1/conversations/${secret.id}/messages`)
  await refused(member2, 'POST', `/v1/conversations/${secret.id}/messages`, { bodySchemaVersion: 1, body: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'should be refused' }] }] }, threadRootId: null, replyToId: null })
  await refused(outsider, 'GET', `/v1/projects/${projectId}/board`)
  await refused(outsider, 'GET', `/v1/conversations/${room.id}/messages`)
  await refused(admin, 'GET', `/v1/conversations/${secret.id}/messages`)
  await refused(member2, 'PUT', `/v1/projects/${projectId}/grants/${ids.outsider}`, { role: 'ADMIN' })
  await refused(member, 'PUT', `/v1/channels/${secret.id}/members/${ids.member2}`, {})
  await refused(member, 'POST', `/v1/workspaces/${workspaceId}/invitations`, { email: `mu-x-${stamp}@example.test`, role: 'MEMBER' })
})

/** The message texts of a conversation as the page shows them, in order. */
async function shownMessages(page: Page, prefix: string): Promise<string[]> {
  const texts = await page.getByRole('article').allInnerTexts()
  return texts.flatMap((text) => text.split('\n')).filter((line) => line.startsWith(prefix))
}

async function send(actor: Actor, text: string): Promise<Response> {
  const box = actor.page.getByRole('textbox', { name: 'Write a message' })
  await box.click()
  await actor.page.keyboard.type(text)
  const posted = apiResponse(actor.page, 'POST', /\/messages$/)
  await actor.page.getByRole('button', { name: 'Send' }).click()
  return posted
}

test('chat: two people send at the same moment, three browsers show both once and in the same order', async () => {
  const prefix = `race-${stamp}`
  for (const actor of [owner, member, member2]) {
    await actor.act(`open # ${room.name}`, async () => {
      await actor.page.goto(projectUrl(`/chat/${room.id}`))
      await expect(actor.page.getByRole('heading', { name: `# ${room.name}`, level: 1 })).toBeVisible()
    })
  }
  // Both write first, then both press Send in the same tick.
  const typeOnly = async (actor: Actor, text: string) => {
    await actor.page.getByRole('textbox', { name: 'Write a message' }).click()
    await actor.page.keyboard.type(text)
  }
  await typeOnly(owner, `${prefix} from owner`)
  await typeOnly(member, `${prefix} from member`)
  const [ownerPost, memberPost] = await together(
    () => owner.act('press Send', async () => {
      const posted = apiResponse(owner.page, 'POST', /\/messages$/)
      await owner.page.getByRole('button', { name: 'Send' }).click()
      return posted
    }),
    () => member.act('press Send', async () => {
      const posted = apiResponse(member.page, 'POST', /\/messages$/)
      await member.page.getByRole('button', { name: 'Send' }).click()
      return posted
    }),
  )
  expect(ownerPost.status()).toBeLessThan(300)
  expect(memberPost.status()).toBeLessThan(300)
  // A third message from the WebKit person while the others only watch: it must arrive live.
  expect((await send(member2, `${prefix} from member2`)).status()).toBeLessThan(300)

  const expected = [`${prefix} from owner`, `${prefix} from member`, `${prefix} from member2`]
  const orders: string[][] = []
  for (const actor of [owner, member, member2]) {
    await actor.act('see all three messages live, once each', async () => {
      await expect.poll(async () => (await shownMessages(actor.page, prefix)).sort()).toEqual([...expected].sort())
      orders.push(await shownMessages(actor.page, prefix))
    })
  }
  journal.write('stage', 'chat order', JSON.stringify(orders))
  expect(orders[1], 'Firefox shows the same order as Chromium').toEqual(orders[0])
  expect(orders[2], 'WebKit shows the same order as Chromium').toEqual(orders[0])
  // The order on screen is the server's order (stream sequence).
  const server = await owner.json<Array<{ streamSeq: number; body: unknown }>>('GET', `/v1/conversations/${room.id}/messages`)
  const serverOrder = [...server].sort((a, b) => a.streamSeq - b.streamSeq)
    .map((item) => JSON.stringify(item.body).match(new RegExp(`${prefix} from \\w+`))?.[0])
    .filter((text): text is string => Boolean(text))
  journal.write('stage', 'server order', JSON.stringify(serverOrder))
  expect(orders[0], 'the screens show the server order').toEqual(serverOrder)
})


const ticketPanel = (page: Page) => page.getByRole('complementary', { name: 'Ticket detail' })
/** Opens a ticket and waits until the page's Project socket is subscribed, so later changes reach it live. */
const openTicket = (actor: Actor, ticketId: string) => actor.act(`open ticket ${ticketId}`, async () => {
  const ready = projectSocketReady(actor.page)
  await actor.page.goto(projectUrl(`/work?panel=ticket:${ticketId}`))
  await expect(ticketPanel(actor.page).getByRole('textbox', { name: 'Title' })).toBeVisible()
  await ready
})
type Ticket = { id: string; key: string; title: string; revisionNo: number; statusId: string; priority?: string | null; description?: unknown; archivedAt?: string | null }

/** Whether a condition shows up on its own (a live update) within a short time; the answer is logged, not asserted. */
async function showsLive(actor: Actor, what: string, check: () => Promise<void>): Promise<boolean> {
  try {
    await check()
    journal.write(actor.tag, 'live', `${what}: shown without reload`)
    return true
  } catch {
    journal.write(actor.tag, 'live', `${what}: NOT shown without reload`)
    return false
  }
}

test('ticket: two people change different fields of one ticket at the same moment; nothing is lost', async () => {
  const ticket = await owner.json<Ticket>('POST', `/v1/projects/${projectId}/tickets`, { title: `MU race ${stamp}` })
  await Promise.all([openTicket(owner, ticket.id), openTicket(member, ticket.id), openTicket(member2, ticket.id)])
  // A refused stale write is the expected protection; its 409 is logged as expected.
  for (const actor of [owner, member]) actor.allow(/^409 /)

  const [stateWrite, priorityWrite] = await together(
    () => owner.act('set State to In Progress', async () => {
      const written = owner.page.waitForResponse((response) => response.request().method() !== 'GET' && response.url().includes(`/tickets/${ticket.id}`))
      await ticketPanel(owner.page).getByRole('combobox', { name: 'State', exact: true }).selectOption({ label: 'In Progress' })
      return written
    }),
    () => member.act('set Priority to High', async () => {
      const written = member.page.waitForResponse((response) => response.request().method() !== 'GET' && response.url().includes(`/tickets/${ticket.id}`))
      await ticketPanel(member.page).getByRole('combobox', { name: 'Priority' }).selectOption({ label: 'High' })
      return written
    }),
  )
  journal.write('stage', 'field race', `state ${stateWrite.status()} ${stateWrite.url()}; priority ${priorityWrite.status()} ${priorityWrite.url()}`)

  // A write refused as stale must say so. A field edit keeps the input and offers "Re-apply on latest"; a State
  // change is a command: the screen says the ticket changed first, shows the latest State, and the person does it again.
  if (priorityWrite.status() === 409) {
    await member.act('see the field conflict and re-apply on the latest', async () => {
      await expect(member.page.getByText('Someone else changed this ticket first')).toBeVisible()
      await member.page.getByRole('button', { name: 'Re-apply on latest' }).click()
      await expect(member.page.getByText('Someone else changed this ticket first')).toHaveCount(0)
    })
  }
  if (stateWrite.status() === 409) {
    await owner.act('see the State change refused, then set it again', async () => {
      const panel = ticketPanel(owner.page)
      await expect(owner.page.getByText('Someone changed this ticket first. The board was reloaded.')).toBeVisible()
      await expect.poll(() => panel.getByRole('combobox', { name: 'State', exact: true }).evaluate((el: HTMLSelectElement) => el.selectedOptions[0]?.text)).toBe('Todo')
      const again = owner.page.waitForResponse((r) => r.request().method() === 'POST' && r.url().includes(`/tickets/${ticket.id}/commands/change-status`))
      await panel.getByRole('combobox', { name: 'State', exact: true }).selectOption({ label: 'In Progress' })
      expect((await again).status()).toBeLessThan(300)
    })
  }
  await expect.poll(async () => {
    const latest = await owner.json<Ticket & { priority: string }>('GET', `/v1/tickets/${ticket.id}`)
    return latest.priority
  }).toBe('HIGH')
  for (const actor of [owner, member, member2]) {
    const panel = ticketPanel(actor.page)
    const live = await showsLive(actor, 'State In Progress and Priority High', async () => {
      await expect(panel.getByRole('combobox', { name: 'State', exact: true })).toHaveValue(/.+/, { timeout: 1 })
      await expect.poll(async () => [
        await panel.getByRole('combobox', { name: 'State', exact: true }).evaluate((el: HTMLSelectElement) => el.selectedOptions[0]?.text),
        await panel.getByRole('combobox', { name: 'Priority' }).evaluate((el: HTMLSelectElement) => el.selectedOptions[0]?.text),
      ], { timeout: 8_000 }).toEqual(['In Progress', 'High'])
    })
    if (!live) {
      await actor.page.reload()
      await expect(panel.getByRole('combobox', { name: 'Priority' })).toBeVisible()
    }
    await actor.act('show both changes', async () => {
      await expect.poll(async () => [
        await panel.getByRole('combobox', { name: 'State', exact: true }).evaluate((el: HTMLSelectElement) => el.selectedOptions[0]?.text),
        await panel.getByRole('combobox', { name: 'Priority' }).evaluate((el: HTMLSelectElement) => el.selectedOptions[0]?.text),
      ]).toEqual(['In Progress', 'High'])
    })
  }
})

test('ticket: two people rename one ticket at the same moment; one wins, the other is told, all agree', async () => {
  const ticket = await owner.json<Ticket>('POST', `/v1/projects/${projectId}/tickets`, { title: `MU title ${stamp}` })
  await Promise.all([openTicket(owner, ticket.id), openTicket(member, ticket.id), openTicket(member2, ticket.id)])
  for (const actor of [owner, member]) actor.allow(/^409 /)
  const titles = { owner: `Owner title ${stamp}`, member: `Member title ${stamp}` }
  await ticketPanel(owner.page).getByRole('textbox', { name: 'Title' }).fill(titles.owner)
  await ticketPanel(member.page).getByRole('textbox', { name: 'Title' }).fill(titles.member)
  const [ownerWrite, memberWrite] = await together(
    () => owner.act('press Enter in Title', async () => {
      const written = owner.page.waitForResponse((r) => r.request().method() === 'PATCH' && r.url().includes(`/tickets/${ticket.id}`))
      await ticketPanel(owner.page).getByRole('textbox', { name: 'Title' }).press('Enter')
      return written
    }),
    () => member.act('press Enter in Title', async () => {
      const written = member.page.waitForResponse((r) => r.request().method() === 'PATCH' && r.url().includes(`/tickets/${ticket.id}`))
      await ticketPanel(member.page).getByRole('textbox', { name: 'Title' }).press('Enter')
      return written
    }),
  )
  const statuses = [ownerWrite.status(), memberWrite.status()]
  journal.write('stage', 'title race', `owner ${statuses[0]}, member ${statuses[1]}`)
  expect(statuses.filter((status) => status < 300), 'exactly one rename is saved').toHaveLength(1)
  expect(statuses.filter((status) => status === 409), 'the other is refused as stale').toHaveLength(1)
  const winner = statuses[0] < 300 ? 'owner' : 'member'
  const loser = winner === 'owner' ? member : owner
  await loser.act('see the conflict with both values and keep the latest', async () => {
    await expect(loser.page.getByText('Someone else changed this ticket first')).toBeVisible()
    await expect(loser.page.getByText(titles[winner]).first()).toBeVisible()
    await loser.page.getByRole('button', { name: 'Discard mine' }).click()
    await expect(ticketPanel(loser.page).getByRole('textbox', { name: 'Title' })).toHaveValue(titles[winner])
  })
  const saved = await owner.json<Ticket>('GET', `/v1/tickets/${ticket.id}`)
  expect(saved.title).toBe(titles[winner])
  for (const actor of [owner, member, member2]) {
    const title = ticketPanel(actor.page).getByRole('textbox', { name: 'Title' })
    const live = await showsLive(actor, `title "${titles[winner]}"`, () => expect(title).toHaveValue(titles[winner], { timeout: 8_000 }))
    if (!live) {
      await actor.page.reload()
      await expect(title).toHaveValue(titles[winner])
    }
  }
})

// Finding MU-1 (since v0.1.0, fixed in v0.2.1): a change committed after a page read its open Ticket but before its
// Project socket said `project.ready` was not shown in that open Ticket panel (the Board beside it was read again on
// `project.ready`, the open Tickets were not). The socket is held back to make the gap certain instead of relying on
// timing.
test('an open ticket catches up with a change made while its Project socket was connecting', async () => {
  const ticket = await owner.json<Ticket>('POST', `/v1/projects/${projectId}/tickets`, { title: `MU gap before ${stamp}` })
  let release!: () => void
  const held = new Promise<void>((resolve) => { release = resolve })
  await member2.page.routeWebSocket(/\/v1\/ws\/projects\//, async (route) => {
    await held
    route.connectToServer()
  })
  try {
    await member2.act('open the ticket while the Project socket is held back', async () => {
      await member2.page.goto(projectUrl(`/work?panel=ticket:${ticket.id}`))
      await expect(ticketPanel(member2.page).getByRole('textbox', { name: 'Title' })).toHaveValue(`MU gap before ${stamp}`)
    })
    await owner.json('PATCH', `/v1/tickets/${ticket.id}`, { expectedRevision: ticket.revisionNo, title: `MU gap after ${stamp}` })
    const ready = projectSocketReady(member2.page)
    release()
    await ready
    await member2.act('see the Board card and the open panel agree', async () => {
      await expect(member2.page.getByRole('button', { name: `Open ${ticket.key}` })).toContainText(`MU gap after ${stamp}`)
      await expect(ticketPanel(member2.page).getByRole('textbox', { name: 'Title' })).toHaveValue(`MU gap after ${stamp}`, { timeout: 8_000 })
    })
  } finally {
    release()
    await member2.page.unrouteAll({ behavior: 'ignoreErrors' })
  }
})

test('ticket: archived by the Project admin while others have it open and one is writing a comment', async () => {
  // Archiving is for Project admins (editors get no Archive button), so the owner archives; member writes a comment in
  // the open panel and member2 watches the Board.
  const ticket = await owner.json<Ticket>('POST', `/v1/projects/${projectId}/tickets`, { title: `MU archive ${stamp}` })
  await member2.act('open the Board', async () => {
    const ready = projectSocketReady(member2.page)
    await member2.page.goto(projectUrl('/work'))
    await expect(member2.page.getByRole('button', { name: `Open ${ticket.key}` })).toBeVisible()
    await ready
  })
  await Promise.all([openTicket(owner, ticket.id), openTicket(member, ticket.id)])
  await expect(ticketPanel(member.page).getByRole('button', { name: 'Archive', exact: true }), 'an editor is not offered Archive').toHaveCount(0)
  const comment = `comment while archiving ${stamp}`
  await ticketPanel(member.page).getByRole('textbox', { name: 'Write a comment' }).fill(comment)
  member.allow(/^4\d\d /)
  await ticketPanel(owner.page).getByRole('button', { name: 'Archive', exact: true }).click()
  const confirm = owner.page.getByRole('dialog').filter({ has: owner.page.getByRole('button', { name: 'Archive', exact: true }) }).last()
  await expect(confirm).toBeVisible()
  const [archived, commented] = await together(
    () => owner.act('confirm Archive', async () => {
      const done = owner.page.waitForResponse((r) => r.request().method() === 'POST' && r.url().includes(`/tickets/${ticket.id}/commands/archive`))
      await confirm.getByRole('button', { name: 'Archive', exact: true }).click()
      return done
    }),
    () => member.act('press Comment', async () => {
      const done = member.page.waitForResponse((r) => r.request().method() === 'POST' && r.url().includes(`/tickets/${ticket.id}/comments`))
      await ticketPanel(member.page).getByRole('button', { name: 'Comment', exact: true }).click()
      return done
    }),
  )
  journal.write('stage', 'archive race', `archive ${archived.status()}, comment ${commented.status()}`)
  expect(archived.status()).toBeLessThan(300)
  const comments = await owner.json<unknown>('GET', `/v1/tickets/${ticket.id}/comments`)
  const commentSaved = JSON.stringify(comments).includes(comment)
  expect(commentSaved, 'the comment is saved exactly when the server said so').toBe(commented.status() < 300)
  if (commented.status() >= 300) {
    await member.act('see that the comment was not saved', async () => {
      const notice = member.page.getByRole('alert').or(member.page.getByRole('status')).filter({ hasText: /\S/ })
      await expect(notice.first()).toBeVisible()
      journal.write(member.tag, 'shown', `after the refused comment: ${JSON.stringify(await notice.allInnerTexts())}`)
    })
  }
  const live = await showsLive(member, 'the open panel turns read-only (archived banner)', () => expect(ticketPanel(member.page).getByRole('note')).toBeVisible({ timeout: 8_000 }))
  if (!live) {
    await member.page.reload()
    await expect(ticketPanel(member.page).getByRole('note')).toBeVisible()
  }
  await expect(ticketPanel(member.page).getByRole('textbox', { name: 'Write a comment' }), 'an archived ticket takes no new comment').toHaveCount(0)
  const card = member2.page.getByRole('button', { name: `Open ${ticket.key}` })
  const gone = await showsLive(member2, 'archived ticket leaves the Board', () => expect(card).toHaveCount(0, { timeout: 8_000 }))
  if (!gone) {
    await member2.page.reload()
    await expect(card).toHaveCount(0)
  }
  const latest = await owner.json<Ticket>('GET', `/v1/tickets/${ticket.id}`)
  expect(latest.archivedAt).toBeTruthy()
})

async function editDescription(actor: Actor, text: string) {
  const panel = ticketPanel(actor.page)
  await panel.getByRole('button', { name: 'Edit description' }).click()
  const editor = panel.locator('[contenteditable="true"]').first()
  await editor.click()
  await actor.page.keyboard.type(text)
}

test('ticket description: two people save their edits at the same moment; the later save is refused, not merged away', async () => {
  const ticket = await owner.json<Ticket>('POST', `/v1/projects/${projectId}/tickets`, { title: `MU description ${stamp}` })
  await Promise.all([openTicket(owner, ticket.id), openTicket(member, ticket.id), openTicket(member2, ticket.id)])
  for (const actor of [owner, member]) actor.allow(/^409 /)
  const texts = { owner: `owner wrote this ${stamp}`, member: `member wrote this ${stamp}` }
  await editDescription(owner, texts.owner)
  await editDescription(member, texts.member)
  const save = (actor: Actor) => actor.act('press Save under the description', async () => {
    const done = actor.page.waitForResponse((r) => r.request().method() === 'PATCH' && r.url().includes(`/tickets/${ticket.id}`))
    await ticketPanel(actor.page).getByRole('button', { name: 'Save', exact: true }).click()
    return done
  })
  const [ownerSave, memberSave] = await together(() => save(owner), () => save(member))
  const statuses = [ownerSave.status(), memberSave.status()]
  journal.write('stage', 'description race', `owner ${statuses[0]}, member ${statuses[1]}`)
  expect(statuses.filter((status) => status < 300)).toHaveLength(1)
  expect(statuses.filter((status) => status === 409)).toHaveLength(1)
  const winner = statuses[0] < 300 ? 'owner' : 'member'
  const loserName = winner === 'owner' ? 'member' : 'owner'
  const loser = winner === 'owner' ? member : owner
  const saved = JSON.stringify((await owner.json<Ticket>('GET', `/v1/tickets/${ticket.id}`)).description)
  expect(saved).toContain(texts[winner])
  expect(saved).not.toContain(texts[loserName])
  await loser.act('be told the save was refused and still have the draft', async () => {
    await expect(loser.page.getByText(/A newer change was saved first|Someone else changed this ticket first/).first()).toBeVisible()
    await expect(ticketPanel(loser.page).locator('[contenteditable="true"]').first()).toContainText(texts[loserName])
  })
  const viewer = ticketPanel(member2.page)
  const live = await showsLive(member2, 'the saved description', () => expect(viewer.getByText(texts[winner])).toBeVisible({ timeout: 8_000 }))
  if (!live) {
    await member2.page.reload()
    await expect(viewer.getByText(texts[winner])).toBeVisible()
  }
})

test('document: two people save the same page at the same moment while a third reads it', async () => {
  const doc = await owner.json<{ id: string }>('POST', `/v1/workspaces/${workspaceId}/docs`, { title: `MU page ${stamp}`, visibility: 'WORKSPACE' })
  const open = (actor: Actor) => actor.act('open the page', async () => {
    await actor.page.goto(w(`/docs?doc=${doc.id}`))
    await expect(actor.page.getByRole('button', { name: 'Edit' })).toBeVisible()
  })
  await Promise.all([open(owner), open(member), open(member2)])
  for (const actor of [owner, member]) actor.allow(/^409 /)
  const texts = { owner: `owner page text ${stamp}`, member: `member page text ${stamp}` }
  for (const [actor, text] of [[owner, texts.owner], [member, texts.member]] as const) {
    await actor.act('start editing and type', async () => {
      await actor.page.getByRole('button', { name: 'Edit' }).click()
      await actor.page.locator('.ProseMirror[contenteditable="true"]').first().click()
      await actor.page.keyboard.type(text)
      await expect(actor.page.getByRole('status').filter({ hasText: 'Unsaved changes' })).toBeVisible()
    })
  }
  const save = (actor: Actor) => actor.act('press Save', async () => {
    const done = actor.page.waitForResponse((r) => ['PUT', 'PATCH', 'POST'].includes(r.request().method()) && r.url().includes(`/docs/${doc.id}`))
    await actor.page.getByRole('button', { name: 'Save' }).click()
    return done
  })
  const [ownerSave, memberSave] = await together(() => save(owner), () => save(member))
  const statuses = [ownerSave.status(), memberSave.status()]
  journal.write('stage', 'document race', `owner ${statuses[0]} ${ownerSave.url()}, member ${statuses[1]} ${memberSave.url()}`)
  expect(statuses.filter((status) => status < 300)).toHaveLength(1)
  expect(statuses.filter((status) => status === 409)).toHaveLength(1)
  const winner = statuses[0] < 300 ? 'owner' : 'member'
  const loserName = winner === 'owner' ? 'member' : 'owner'
  const loser = winner === 'owner' ? member : owner
  const content = JSON.stringify((await owner.json<{ content: unknown }>('GET', `/v1/docs/${doc.id}`)).content)
  expect(content).toContain(texts[winner])
  expect(content).not.toContain(texts[loserName])
  await loser.act('be told the save was refused, draft kept', async () => {
    await expect(loser.page.getByText('Someone saved this page first').first()).toBeVisible()
    await expect(loser.page.locator('.ProseMirror[contenteditable="true"]').first()).toContainText(texts[loserName])
  })
  const reader = member2.page
  const live = await showsLive(member2, 'the saved page text', () => expect(reader.getByText(texts[winner])).toBeVisible({ timeout: 8_000 }))
  if (!live) {
    await reader.reload()
    await expect(reader.getByText(texts[winner])).toBeVisible()
  }
})

test('mentions and read state belong to each person', async () => {
  type Inbox = { items: Array<{ id: string; type?: string; kind?: string; title?: string; summary?: unknown } & Record<string, unknown>> }
  const inbox = async (actor: Actor) => JSON.stringify(await actor.json<Inbox>('GET', `/v1/workspaces/${workspaceId}/inbox?view=inbox`))
  const unread = async (actor: Actor) => {
    const space = await actor.json<{ channels: Array<{ id: string; unreadCount: number }> }>('GET', `/v1/workspaces/${workspaceId}/space`)
    return space.channels.find((channel) => channel.id === room.id)?.unreadCount ?? -1
  }
  // member and member2 look elsewhere so the room collects unread messages for them.
  await Promise.all([member, member2].map((actor) => actor.act('go to Home', async () => {
    await actor.page.goto(w())
    await expect(actor.page.getByRole('heading', { name: workspaceName, level: 1 })).toBeVisible()
  })))
  const before = { member: await unread(member), member2: await unread(member2), owner: await unread(owner) }
  const marker = `mention-${stamp}`
  await owner.act('mention MU Member in the room', async () => {
    await owner.page.goto(projectUrl(`/chat/${room.id}`))
    const box = owner.page.getByRole('textbox', { name: 'Write a message' })
    await box.click()
    await owner.page.keyboard.type(`${marker} hello `)
    await owner.page.keyboard.type('@MU')
    await expect(owner.page.getByRole('listbox', { name: 'People to mention' })).toBeVisible()
    await owner.page.locator('.mention-option').filter({ has: owner.page.locator('.mention-name', { hasText: /^MU Member$/ }) }).click()
    await expect(owner.page.getByRole('listbox', { name: 'People to mention' })).toHaveCount(0)
    const posted = apiResponse(owner.page, 'POST', /\/messages$/)
    await owner.page.getByRole('button', { name: 'Send' }).click()
    expect((await posted).status()).toBeLessThan(300)
  })
  await expect.poll(() => inbox(member)).toContain(marker)
  expect(await inbox(member2), 'member2 was not mentioned').not.toContain(marker)
  expect(await inbox(owner), 'the author gets no inbox item for their own mention').not.toContain(marker)
  await expect.poll(() => unread(member)).toBe(before.member + 1)
  await expect.poll(() => unread(member2)).toBe(before.member2 + 1)
  expect(await unread(owner), 'the author has nothing unread').toBe(0)

  await member.act('see the mention in the Inbox (live) and open the room', async () => {
    await member.page.goto(w('/inbox'))
    await expect(member.page.getByText(new RegExp(marker)).first()).toBeVisible()
    await member.page.goto(projectUrl(`/chat/${room.id}`))
    await expect(member.page.getByRole('article').filter({ hasText: marker })).toBeVisible()
  })
  await expect.poll(() => unread(member)).toBe(0)
  expect(await unread(member2), 'member reading the room leaves member2 unread').toBe(before.member2 + 1)
  await member2.act('find the mention absent from the own Inbox', async () => {
    await member2.page.goto(w('/inbox'))
    await expect(member2.page.getByRole('heading', { name: /Inbox/ }).first()).toBeVisible()
    await expect(member2.page.getByText(new RegExp(marker))).toHaveCount(0)
  })
})

test('private channel membership and Project participation change while the person looks', async () => {
  await member.act('open the private channel', async () => {
    await member.page.goto(projectUrl(`/chat/${secret.id}`))
    await expect(member.page.getByRole('heading', { name: `# ${secret.name}`, level: 1 })).toBeVisible()
  })
  member.allow(/^40[34] /)
  // MU-4 (low): the conversation socket, closed by the removal, may try once more before the page notices.
  member.allow(/Firefox can.t establish a connection to the server at ws:\/\/.*\/v1\/ws\?conversationId=/)
  member.allow(/The connection to ws:\/\/.*\/v1\/ws\?conversationId=.* was interrupted while the page was loading/)
  // The member writes while the owner removes them: either the message got in before (and the owner sees it) or it
  // is refused (and nobody sees it). Never "sent" on one screen and missing on the other.
  const text = `racing the removal ${stamp}`
  await member.page.getByRole('textbox', { name: 'Write a message' }).click()
  await member.page.keyboard.type(text)
  await owner.page.goto(projectUrl(`/chat/${secret.id}?settings=1&members=1`))
  const dialog = owner.page.getByRole('dialog', { name: 'Add people to this channel' })
  const row = dialog.getByRole('list', { name: 'Members' }).getByRole('listitem').filter({ hasText: people.member.name })
  await row.getByRole('button', { name: 'Remove' }).click()
  const confirm = owner.page.getByRole('alertdialog').or(owner.page.getByRole('dialog').filter({ hasText: 'Remove MU Member from this channel?' })).last()
  await expect(confirm).toBeVisible()
  const [removed, sent] = await together(
    () => owner.act('confirm removing MU Member', async () => {
      const done = apiResponse(owner.page, 'DELETE', /\/channels\/.*\/members\//)
      await confirm.getByRole('button', { name: 'Remove' }).click()
      return done
    }),
    () => member.act('press Send', async () => {
      const done = apiResponse(member.page, 'POST', /\/messages$/)
      await member.page.getByRole('button', { name: 'Send' }).click()
      return done
    }),
  )
  journal.write('stage', 'removal race', `remove ${removed.status()}, send ${sent.status()}`)
  expect(removed.status()).toBeLessThan(300)
  const messages = JSON.stringify(await owner.json<unknown>('GET', `/v1/conversations/${secret.id}/messages`))
  expect(messages.includes(text), 'the message exists exactly when the send succeeded').toBe(sent.status() < 300)

  const lost = await showsLive(member, 'losing the private channel', () => expect(notFound(member.page)).toBeVisible({ timeout: 8_000 }))
  if (!lost) {
    await member.page.reload()
    await expect(notFound(member.page)).toBeVisible()
  }
  await expect(sidebar(member.page).getByRole('link', { name: `# ${secret.name}` })).toHaveCount(0)
  const refused = await member.api('GET', `/v1/conversations/${secret.id}/messages`)
  expect([403, 404]).toContain(refused.status())

  // Project participation: member2 is let out of the Project while looking at its channel, then let in again.
  member2.allow(/^40[34] /)
  // The sockets the removal closed are refused when they try again (a browser logs that) until the page has noticed.
  member2.allow(/Firefox can.t establish a connection to the server at ws:\/\/.*\/v1\/ws/)
  member2.allow(/The connection to ws:\/\/.*\/v1\/ws.* was interrupted while the page was loading/)
  member2.allow(/WebSocket connection to 'ws:\/\/.*\/v1\/ws.*' failed/)
  await member2.act('open the Project-members channel', async () => {
    await member2.page.goto(projectUrl(`/chat/${room.id}`))
    await expect(member2.page.getByRole('heading', { name: `# ${room.name}`, level: 1 })).toBeVisible()
  })
  await owner.act('remove MU Member Two from the Project', async () => {
    await owner.page.goto(projectUrl('/settings'))
    const permissions = owner.page.getByRole('region', { name: 'Permissions' })
    const done = apiResponse(owner.page, 'DELETE', /\/grants\//)
    await permissions.getByRole('button', { name: `Remove ${people.member2.name}` }).click()
    const ask = owner.page.getByRole('alertdialog').or(owner.page.getByRole('dialog')).last()
    if (await ask.isVisible().catch(() => false)) await ask.getByRole('button', { name: /^Remove/ }).click()
    expect((await done).status()).toBeLessThan(300)
  })
  // Finding MU-2 (since v0.1.0, fixed in v0.2.1): a person who read the channel only through their Project role holds
  // no channel membership, and nothing told their open page. The removal now closes their sockets and has their
  // Space read again, so the page says it at once instead of keeping a channel that refuses every message.
  await member2.act('see the Project channel go without a reload', async () => {
    await expect(member2.page.getByText('You are not part of this project.').or(notFound(member2.page)).first()).toBeVisible({ timeout: 8_000 })
    await expect(member2.page.getByRole('textbox', { name: 'Write a message' })).toHaveCount(0)
    await expect(sidebar(member2.page).getByRole('link', { name: `# ${room.name}` })).toHaveCount(0)
  })
  expect([403, 404]).toContain((await member2.api('GET', `/v1/conversations/${room.id}/messages`)).status())

  await owner.act('give MU Member Two the Editor role again', async () => {
    const permissions = owner.page.getByRole('region', { name: 'Permissions' })
    await permissions.getByRole('combobox', { name: 'Role to give' }).selectOption({ label: 'Editor' })
    const done = apiResponse(owner.page, 'PUT', /\/grants\//)
    await permissions.getByRole('listitem').getByRole('button', { name: people.member2.name, exact: true }).click()
    expect((await done).status()).toBe(204)
  })
  await member2.act('read the Project-members channel again (it follows participation)', async () => {
    await member2.page.goto(projectUrl(`/chat/${room.id}`))
    await expect(member2.page.getByRole('heading', { name: `# ${room.name}`, level: 1 })).toBeVisible()
    await expect(member2.page.getByRole('article').first()).toBeVisible()
  })
})

// Finding MU-3 (since v0.1.0, fixed in v0.2.1): the header of a private Project channel counted and pictured the
// Project's members, not the channel's, so member2, who cannot open the channel, was shown as one of its members.
test('a private channel header counts the channel members, not the Project members', async () => {
  const members = await owner.json<{ items: unknown[] }>('GET', `/v1/channels/${secret.id}/members`)
  await owner.act('read the private channel header', async () => {
    await owner.page.goto(projectUrl(`/chat/${secret.id}`))
    const header = owner.page.locator('header.context-bar')
    await expect(header.getByRole('heading', { name: `# ${secret.name}`, level: 1 })).toBeVisible()
    journal.write(owner.tag, 'shown', `header: ${await header.locator('p').first().innerText()}; channel members ${members.items.length}`)
    await expect(header.locator('p').first()).toContainText(`${members.items.length} members`, { timeout: 3_000 })
  })
})

test('signing out in one browser ends only that session', async () => {
  // Finding MU-4 (low, not an application fault): signing out within a moment of the page loading, while the user
  // socket's first handshake is still on its way, has that handshake refused (the session is gone) and Firefox logs
  // it. A person signs out from a page that has settled, so the test does too, and then allows no socket error.
  const second = await stage.enter(people.member, member.engine === 'chromium' ? 'webkit' : 'chromium')
  await expect(second.page.getByRole('button', { name: people.member.name, exact: true })).toBeVisible()
  await member.act('sign out from the account menu', async () => {
    const settled = userSocketReady(member.page)
    await member.page.goto(w())
    await settled
    await member.page.getByRole('button', { name: people.member.name, exact: true }).click()
    await member.page.getByRole('button', { name: 'Sign out' }).click()
    await expect(member.page).toHaveURL(/\/login/)
  })
  member.allow(/^401 /)
  expect((await member.api('GET', '/v1/me')).status(), 'the signed-out session is gone').toBe(401)
  const after = await Promise.all([second, owner, member2, outsider, admin].map(async (actor) => {
    const me = await actor.api('GET', '/v1/me')
    return `${actor.tag}:${me.status()}`
  }))
  journal.write('stage', 'after sign-out', after.join(' '))
  expect(after.every((entry) => entry.endsWith(':200')), 'every other session, the same person on another browser included, still works').toBe(true)
  await together(
    () => second.act('keep working as MU Member in Chromium', async () => {
      await second.page.goto(projectUrl(`/chat/${room.id}`))
      expect((await send(second, `still here ${stamp}`)).status()).toBeLessThan(300)
    }),
    () => owner.act('keep working', async () => {
      await owner.page.goto(projectUrl('/work'))
      await expect(owner.page.getByRole('region', { name: 'Todo' })).toBeVisible()
    }),
    () => member2.act('keep working and see the message', async () => {
      await member2.page.goto(projectUrl(`/chat/${room.id}`))
      await expect(member2.page.getByRole('article').filter({ hasText: `still here ${stamp}` })).toBeVisible()
    }),
  )
  // What one person opened does not move another: each page stays where its own person left it.
  expect(new URL(owner.page.url()).pathname).toMatch(/\/work$/)
  expect(new URL(member2.page.url()).pathname).toContain(`/chat/${room.id}`)
  await member.logIn()
})
