import type { APIRequestContext } from '@playwright/test'
import { expect, test } from '@playwright/test'
import { accountA } from '../support/accounts'
import { Api } from '../support/api'

const candidate = (n: number) => n.toString(16).padStart(9, '0').padEnd(32, '0')

test('an occupied prefix is replaced on the next attempt', async ({ request }) => {
  const start = Math.floor(Math.random() * 0x100000000)
  const owner = new Api(request, () => candidate(start))
  await owner.login(accountA.email)
  const workspace = await owner.workspaceId()
  await owner.createProject(workspace, 'Occupy a prefix')

  let attempts = 0
  const other = new Api(request, () => candidate(start + attempts++))
  const project = await other.createProject(workspace, 'Retry an occupied prefix')
  const ticket = await other.createTicket(project.id, 'First ticket')
  expect(attempts).toBe(2)
  expect(ticket.key).toBe(`E${candidate(start + 1).slice(0, 9).toUpperCase()}-001`)
})

test('two concurrent project creations retry a real prefix conflict', async ({ request }) => {
  const start = Math.floor(Math.random() * 0x100000000)
  const leftAttempts: number[] = []
  const rightAttempts: number[] = []
  const next = (attempts: number[], offset: number) => () => {
    const n = attempts.length ? start + offset : start
    attempts.push(n)
    return candidate(n)
  }
  const owner = new Api(request, next(leftAttempts, 1))
  await owner.login(accountA.email)
  const workspace = await owner.workspaceId()
  const projects = await Promise.all([
    owner.createProject(workspace, 'Prefix race left'),
    new Api(request, next(rightAttempts, 2)).createProject(workspace, 'Prefix race right'),
  ])

  expect(projects[0].id).not.toBe(projects[1].id)
  expect([leftAttempts.length, rightAttempts.length].sort()).toEqual([1, 2])
  for (const [project, attempts] of [[projects[0], leftAttempts], [projects[1], rightAttempts]] as const) {
    const ticket = await owner.createTicket(project.id, 'First ticket')
    const finalCandidate = attempts[attempts.length - 1]
    expect(ticket.key).toBe(`E${candidate(finalCandidate).slice(0, 9).toUpperCase()}-001`)
  }
})

for (const scenario of [
  { label: 'prefix collision budget is exhausted', status: 409, code: 'ticket_prefix_taken', tries: 5 },
  { label: 'an unrelated conflict fails immediately', status: 409, code: 'project_name_taken', tries: 1 },
  { label: 'a server error fails immediately', status: 500, code: 'ticket_prefix_taken', tries: 1 },
  { label: 'an uncertain transport error fails immediately', status: 0, code: 'connection reset', tries: 1 },
]) {
  test(scenario.label, async () => {
    const prefixes: string[] = []
    const keys: string[] = []
    const request = {
      fetch: async (_path: string, options: { data: { ticketPrefix: string }; headers: Record<string, string> }) => {
        prefixes.push(options.data.ticketPrefix)
        keys.push(options.headers['idempotency-key'])
        if (!scenario.status) throw new Error(scenario.code)
        return { ok: () => false, status: () => scenario.status, text: async () => JSON.stringify({ code: scenario.code }) }
      },
    } as unknown as APIRequestContext
    const api = new Api(request, () => candidate(prefixes.length))

    await expect(api.createProject('workspace', 'Project')).rejects.toThrow(scenario.code)
    expect(prefixes).toHaveLength(scenario.tries)
    expect(new Set(prefixes).size).toBe(scenario.tries)
    expect(new Set(keys).size).toBe(scenario.tries)
    for (const prefix of prefixes) expect(prefix).toMatch(/^[A-Z][A-Z0-9]{1,9}$/)
  })
}
