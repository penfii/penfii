import { expect, test } from '../support/test'
import { accountA } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

// Every main screen opens without a console error, an uncaught error or a failing request (the `test` watch fails the
// test otherwise), with its own heading, and Planning and Timeline follow a change that no ticket frame announces.
test.describe('the main screens', () => {
  test('open one after another without problems', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E screens ${Date.now()}`)
    await api.createTicket(project.id, 'Something to show')
    await logIn(page, accountA.email)

    const base = `/w/${workspaceId}`
    const paths = [
      base,
      `${base}/inbox`,
      `${base}/chat`,
      `${base}/projects`,
      `${base}/projects/${project.id}`,
      `${base}/projects/${project.id}/work`,
      `${base}/projects/${project.id}/planning`,
      `${base}/projects/${project.id}/timeline`,
      `${base}/projects/${project.id}/docs`,
      `${base}/projects/${project.id}/chat`,
      `${base}/projects/${project.id}/settings`,
      `${base}/docs`,
      `${base}/directory`,
      `${base}/search`,
      `${base}/my-work`,
      `${base}/saved`,
      `${base}/drafts`,
      `${base}/settings`,
      `${base}/settings/admin/members`,
      `${base}/settings/admin/audit`,
    ]
    for (const path of paths) {
      await page.goto(path)
      await expect(page.getByRole('heading').first(), path).toBeVisible()
      await expect(page.getByText('Not found'), `${path} is a real page`).toHaveCount(0)
      // The page settles (its first requests finish) before the next one is opened.
      await page.waitForLoadState('networkidle')
    }
  })

  test('Planning and Timeline show a Sprint made elsewhere without a reload', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E sprint ${Date.now()}`)
    await api.createTicket(project.id, 'Scheduled work')
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/planning`)
    await expect(page.getByText(/No Sprints yet/)).toBeVisible()

    // A Sprint is not a ticket change, so the board's change feed has no frame for it: the server announces
    // it for the whole project and the open pages read their data again.
    const name = `Sprint ${Date.now()}`
    const today = new Date()
    const day = (offset: number) => new Date(today.getTime() + offset * 86_400_000).toISOString().slice(0, 10)
    await api.createSprint(project.id, name, day(0), day(13))
    await expect(page.getByText(name).first()).toBeVisible({ timeout: 20_000 })

    await page.goto(`/w/${workspaceId}/projects/${project.id}/timeline`)
    // On the Timeline a Sprint is a band on the axis; its name is the band's tooltip.
    const band = (text: string) => page.locator('svg rect.sprint-band title').filter({ hasText: text })
    await expect(band(name)).toHaveCount(1)
    const second = `Sprint two ${Date.now()}`
    await api.createSprint(project.id, second, day(14), day(27))
    await expect(band(second)).toHaveCount(1, { timeout: 20_000 })
  })

  test('an ordinary build has no demo content and no "Recent projects"', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects`)
    await expect(page.getByRole('heading', { name: 'Projects' }).first()).toBeVisible()
    await expect(page.getByText(/recent projects/i)).toHaveCount(0)
    await expect(page.getByRole('link', { name: /^recent$/i })).toHaveCount(0)
    // The sample projects of the demo build ("FDS", "Platform") exist only there; the real list holds this Workspace's own.
    const real = JSON.stringify(await (await request.get('/v1/workspaces/' + workspaceId + '/space', { headers: { 'x-penfii-client': '1' } })).json())
    for (const sample of ['FDS', 'Platform']) {
      if (!real.includes(`"name":"${sample}"`)) await expect(page.getByRole('link', { name: sample, exact: true })).toHaveCount(0)
    }
    // And the first-load code carries none of it.
    const scripts = await page.evaluate(() => performance.getEntriesByType('resource').map((entry) => entry.name).filter((name) => name.endsWith('.js')))
    for (const url of scripts) expect(url, 'no sample chunk is loaded').not.toMatch(/\/sample-/)
  })

  test('a Milestone renamed and a Sprint started and completed elsewhere reach the open Planning page', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `E2E planning sync ${Date.now()}`)
    const milestoneName = `Launch ${Date.now()}`
    const milestone = await api.createMilestone(project.id, milestoneName)
    const day = (offset: number) => new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10)
    const sprintName = `Run ${Date.now()}`
    const sprint = await api.createSprint(project.id, sprintName, day(0), day(13))
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/planning`)
    await expect(page.getByText(sprintName).first()).toBeVisible()

    // A Milestone change is not a ticket change either: the open page learns of it through the project-wide notice.
    await page.getByRole('tab', { name: 'Milestones' }).click()
    await expect(page.getByText(milestoneName).first()).toBeVisible()
    const renamed = `Launch renamed ${Date.now()}`
    await api.renameMilestone(milestone.id, milestone.version, renamed)
    await expect(page.getByText(renamed).first()).toBeVisible({ timeout: 20_000 })

    await page.getByRole('tab', { name: 'Sprints' }).click()
    // Start, then complete with immediate archive (a path with no per-ticket record): the page shows the Sprint as done.
    const sprintRow = page.locator('li, tr, article, section').filter({ hasText: sprintName }).last()
    await expect(sprintRow.getByText('Planned', { exact: true }).first()).toBeVisible()
    const started = await api.sprintCommand(sprint.id, 'start', sprint.version)
    await expect(sprintRow.getByText('Active', { exact: true }).first()).toBeVisible({ timeout: 20_000 })
    await api.sprintCommand(sprint.id, 'complete', started.version, { archive: 'IMMEDIATE' })
    await expect(sprintRow.getByText('Completed', { exact: true }).first()).toBeVisible({ timeout: 20_000 })
  })
})

