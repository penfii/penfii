import { randomUUID } from 'node:crypto'
import { expect, type APIRequestContext, type Browser, type BrowserContext, type Page } from '@playwright/test'
import { PASSWORD } from './accounts'

const HEADERS = { 'x-penfii-client': '1' }

type Json = Record<string, unknown>

/** The API as a signed-in person, for preparing data and reading back what the screens should show. */
export class Api {
  constructor(readonly request: APIRequestContext) {}

  private async call<T>(method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE', path: string, body?: unknown): Promise<T> {
    const response = await this.request.fetch(path, {
      method,
      headers: { ...HEADERS, 'idempotency-key': randomUUID() },
      data: body,
    })
    expect(response.status(), `${method} ${path}: ${await response.text()}`).toBeLessThan(300)
    const text = await response.text()
    return (text ? JSON.parse(text) : undefined) as T
  }

  async login(email: string): Promise<void> {
    await this.call('POST', '/v1/session/login', { email, password: PASSWORD })
  }

  async workspaceId(): Promise<string> {
    const list = await this.call<Array<{ id: string }>>('GET', '/v1/workspaces')
    expect(list.length).toBeGreaterThan(0)
    return list[0].id
  }

  /** A Project of its own, so a test never meets another test's tickets. */
  async createProject(workspaceId: string, name: string): Promise<{ id: string }> {
    const prefix = Array.from({ length: 4 }, () => String.fromCharCode(65 + Math.floor(Math.random() * 26))).join('')
    return this.call('POST', `/v1/workspaces/${workspaceId}/projects`, { name, teamIds: [], ticketPrefix: prefix })
  }

  async createTicket(projectId: string, title: string): Promise<{ id: string; key: string; revisionNo: number; statusId: string }> {
    return this.call('POST', `/v1/projects/${projectId}/tickets`, { title })
  }

  async board(projectId: string): Promise<{ columns: Array<{ id: string; name: string; stateId: string; tickets: Array<{ id: string; title: string; statusId: string }> }> }> {
    return this.call('GET', `/v1/projects/${projectId}/board`)
  }

  async moveTicket(ticketId: string, statusId: string, expectedRevision: number): Promise<unknown> {
    return this.call('POST', `/v1/tickets/${ticketId}/commands/move`, { statusId, expectedRevision })
  }

  /** `conversationId` is the channel's own id unless the server says otherwise (the client reads it the same way). */
  async createSprint(projectId: string, name: string, startsOn: string, endsOn: string): Promise<{ id: string; version: number }> {
    return this.call('POST', `/v1/projects/${projectId}/sprints`, { name, startsOn, endsOn })
  }

  async createMilestone(projectId: string, name: string): Promise<{ id: string; version: number }> {
    return this.call('POST', `/v1/projects/${projectId}/milestones`, { name })
  }

  async renameMilestone(milestoneId: string, version: number, name: string): Promise<{ version: number }> {
    return this.call('PATCH', `/v1/milestones/${milestoneId}`, { expectedVersion: version, name })
  }

  async sprintCommand(sprintId: string, command: 'start' | 'complete', version: number, extra: Record<string, unknown> = {}): Promise<{ version: number }> {
    return this.call('POST', `/v1/sprints/${sprintId}/commands/${command}`, { expectedVersion: version, ...extra })
  }

  async createChannel(workspaceId: string, name: string): Promise<{ id: string; name: string; conversationId: string }> {
    const channel = await this.call<{ id: string; conversationId?: string }>('POST', `/v1/workspaces/${workspaceId}/channels`, { name, visibility: 'PUBLIC' })
    return { id: channel.id, name, conversationId: channel.conversationId ?? channel.id }
  }

  async messages(conversationId: string): Promise<{ items?: Array<{ body: Json | null }> } & Json> {
    return this.call('GET', `/v1/conversations/${conversationId}/messages`)
  }

  async createDoc(workspaceId: string, title: string): Promise<{ id: string; revisionNo: number }> {
    return this.call('POST', `/v1/workspaces/${workspaceId}/docs`, { title, visibility: 'WORKSPACE' })
  }

  async doc(docId: string): Promise<{ title: string; revisionNo: number; content: unknown }> {
    return this.call('GET', `/v1/docs/${docId}`)
  }

  async draft(contextKind: string, contextId: string): Promise<{ payload?: unknown } | null> {
    const response = await this.request.get(`/v1/drafts?contextKind=${contextKind}&contextId=${contextId}`, { headers: HEADERS })
    expect(response.status()).toBeLessThan(300)
    const text = await response.text()
    return text && text !== 'null' ? (JSON.parse(text) as { payload?: unknown }) : null
  }

  async me(): Promise<{ id: string; displayName: string }> {
    return this.call('GET', '/v1/me')
  }
}

/** A browser context that is signed in as `email` through the login screen, as a person does. */
export async function signedInContext(browser: Browser, email: string, watch: (page: Page) => void): Promise<{ context: BrowserContext; page: Page }> {
  const context = await browser.newContext({ locale: 'en-US', serviceWorkers: 'block' })
  const page = await context.newPage()
  watch(page)
  await logIn(page, email)
  return { context, page }
}

export async function logIn(page: Page, email: string): Promise<void> {
  await page.goto('/login')
  await page.getByLabel('Email').fill(email)
  await page.getByLabel('Password').fill(PASSWORD)
  await page.getByRole('button', { name: 'Log in' }).click()
  await expect(page).not.toHaveURL(/\/login/)
}

/** The same sign-in through the API, for tests that only need data. */
export async function apiAs(playwrightRequest: APIRequestContext, email: string): Promise<Api> {
  const api = new Api(playwrightRequest)
  await api.login(email)
  return api
}
