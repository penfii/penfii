/**
 * What the api client says a route answers with must be what `openapi.json` says it answers with. For every call in
 * `packages/api-client/src/index.ts` of the form `request('/v1/…', {…}) as Promise<Type>`, when the API describes the route's
 * success response as a named schema, that schema's name has to be `Type`. A call written against the wrong type, or a route
 * whose description moved to another schema, fails here instead of compiling quietly (the cast hides it from the compiler).
 * Routes the API still describes as "any object" are not checked: that is the part of the API not described yet.
 */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const here = path.dirname(fileURLToPath(import.meta.url))
const openapi = JSON.parse(fs.readFileSync(path.join(here, '..', 'openapi.json'), 'utf8')) as {
  paths: Record<string, Record<string, { responses?: Record<string, { content?: { 'application/json'?: { schema?: { $ref?: string } } } }> }>>
}
const client = fs.readFileSync(path.join(here, '..', '..', 'api-client', 'src', 'index.ts'), 'utf8')

type Call = { path: string; method: string; type: string }

const generic = (route: string) => route.replace(/\$\{[^}]+\}|\{[^}]+\}/g, '{}')

function callsOf(source: string): Call[] {
  const calls: Call[] = []
  const pattern = /request\(`([^`?]+)`(?:,\s*(\{[^\n]*?\}))?\)\s+as\s+Promise<([A-Za-z]+)>/g
  for (const match of source.matchAll(pattern)) {
    const method = /method:\s*'([A-Z]+)'/.exec(match[2] ?? '')?.[1] ?? 'GET'
    calls.push({ path: generic(match[1] ?? ''), method, type: match[3] ?? '' })
  }
  return calls
}

/** The named schema of a route's success response, or null when it has none. */
function described(route: string, method: string): string | null {
  for (const [key, operations] of Object.entries(openapi.paths)) {
    if (generic(key) !== route) continue
    const operation = operations[method.toLowerCase()]
    for (const [status, response] of Object.entries(operation?.responses ?? {})) {
      if (!status.startsWith('2')) continue
      const ref = response.content?.['application/json']?.schema?.$ref
      if (ref) return ref.split('/').pop() ?? null
    }
  }
  return null
}

function problems(calls: Call[]): string[] {
  const found: string[] = []
  for (const call of calls) {
    const schema = described(call.path, call.method)
    if (schema && schema !== call.type) found.push(`${call.method} ${call.path}: the client expects ${call.type}, the API describes ${schema}`)
  }
  return found
}

describe('the api client and the response schemas of openapi.json', () => {
  const calls = callsOf(client)

  it('finds the calls and checks a meaningful number of them against a named schema', () => {
    expect(calls.length).toBeGreaterThan(50)
    expect(calls.filter((call) => described(call.path, call.method) !== null).length).toBeGreaterThanOrEqual(15)
  })

  it('covers the Ticket, Board, Planning, Sprint and Milestone routes the screens depend on', () => {
    const named = new Map(calls.map((call) => [`${call.method} ${call.path}`, described(call.path, call.method)]))
    expect(named.get('GET /v1/tickets/{}')).toBe('Ticket')
    expect(named.get('PATCH /v1/tickets/{}')).toBe('Ticket')
    expect(named.get('POST /v1/projects/{}/tickets')).toBe('Ticket')
    expect(named.get('POST /v1/tickets/{}/commands/move')).toBe('Ticket')
    expect(named.get('POST /v1/tickets/{}/commands/change-status')).toBe('Ticket')
    expect(named.get('GET /v1/projects/{}/board')).toBe('ProjectBoard')
    expect(named.get('GET /v1/projects/{}/planning')).toBe('ProjectPlanning')
    expect(named.get('POST /v1/projects/{}/sprints')).toBe('Sprint')
    expect(named.get('POST /v1/projects/{}/milestones')).toBe('Milestone')
  })

  it('agrees on every route that is described', () => {
    expect(problems(calls)).toEqual([])
  })

  it('notices a call written against the wrong type', () => {
    const wrong = callsOf("return request(`/v1/projects/${projectId}/board`) as Promise<Ticket>")
    expect(problems(wrong)).toEqual(['GET /v1/projects/{}/board: the client expects Ticket, the API describes ProjectBoard'])
  })
})
