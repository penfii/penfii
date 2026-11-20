import { randomUUID } from 'node:crypto'
import type { APIRequestContext } from '@playwright/test'
import { expect, test } from '../support/test'
import { accountA, accountB } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// FEA-007: what a person types into Search against what they get, in the real app and index (search mapping 2):
// English word forms meet by their stem, Korean particles and endings still match, an exact ticket key finds that
// ticket, a project filter narrows, and someone outside the project finds none of it.

async function ticketTitles(request: APIRequestContext, workspaceId: string, q: string): Promise<string[]> {
  const response = await request.get(`/v1/workspaces/${workspaceId}/search`, { params: { q, kind: 'ticket', limit: 50, tz: 'UTC' } })
  expect(response.status()).toBe(200)
  const body = (await response.json()) as { results: Array<{ title: string }> }
  return body.results.map((hit) => hit.title).sort()
}

test.describe('FEA-007: search word forms, keys, filters and access', () => {
  test('English forms, Korean particles, an exact key and a project filter answer in the page; an outsider finds nothing', async ({ page, request, browser }) => {
    test.setTimeout(120_000)
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const tag = randomUUID().slice(0, 6)
    const project = await api.createProject(workspaceId, `QA search forms ${tag}`)
    const english = `Deploying the quarterly reports ${tag}`
    const korean = `서버 배포 일정 정리 ${tag}`
    const other = `Unrelated housekeeping ${tag}`
    await api.createTicket(project.id, english)
    const koreanTicket = await api.createTicket(project.id, korean)
    await api.createTicket(project.id, other)
    const prefix = koreanTicket.key.split('-')[0]
    const inProject = (words: string) => `${words} project:${prefix}`

    // The index follows the new tickets within a moment.
    await expect.poll(() => ticketTitles(request, workspaceId, inProject(tag)), { timeout: 30_000 }).toEqual([english, other, korean].sort())

    // What the index answers for each form (the page shows the same answer below).
    const expected: Array<[string, string[]]> = [
      ['deployed', [english]], // -ing in the text, -ed typed
      ['report', [english]], // plural in the text
      ['reported', [english]],
      ['서버가', [korean]], // a particle on the word
      ['배포', [korean]],
      [koreanTicket.key, [korean]], // the exact key
    ]
    for (const [words, titles] of expected) {
      expect(await ticketTitles(request, workspaceId, inProject(words)), words).toEqual(titles)
    }

    await logIn(page, accountA.email)
    for (const [words, titles] of [expected[0], expected[3], expected[5]]) {
      await page.goto(`/w/${workspaceId}/search?q=${encodeURIComponent(inProject(words))}&tab=ticket`)
      for (const title of titles) await expect(page.getByRole('link', { name: new RegExp(title) }).first()).toBeVisible()
      await expect(page.getByRole('link', { name: new RegExp(other) })).toHaveCount(0)
    }
    // Typed into the box, without a filter, in the All tab.
    await page.goto(`/w/${workspaceId}/search`)
    const box = page.getByRole('textbox', { name: /^Words and filters/ })
    await box.fill(`reported ${tag}`)
    await box.press('Enter')
    await expect(page.getByRole('link', { name: new RegExp(english) }).first()).toBeVisible()

    // Someone in the workspace but not in the project: the same words find nothing of it.
    const context = await browser.newContext()
    const outsider = await context.newPage()
    await logIn(outsider, accountB.email)
    const outsiderRequest = outsider.request
    expect(await ticketTitles(outsiderRequest, workspaceId, tag)).toEqual([])
    await outsider.goto(`/w/${workspaceId}/search?q=${encodeURIComponent(tag)}&tab=ticket`)
    await expect(outsider.getByRole('textbox', { name: /^Words and filters/ })).toHaveValue(tag)
    await expect(outsider.getByRole('link', { name: new RegExp(tag) })).toHaveCount(0)
    await context.close()
  })
})
