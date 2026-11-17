import { randomUUID } from 'node:crypto'
import { appendFileSync, mkdirSync } from 'node:fs'
import { dirname } from 'node:path'
import { PASSWORD } from './accounts'
import { chromium, expect, firefox, webkit, type APIResponse, type Browser, type BrowserContext, type BrowserType, type Page } from '@playwright/test'

/**
 * Several people at once, each in a browser context of their own and, where possible, a browser engine of their own.
 * Every event (action, console, HTTP error, socket error) is written with the person, the engine and a timestamp, so a
 * failure can be told apart as one person's problem or a problem of two people acting together.
 */

export type EngineName = 'chromium' | 'firefox' | 'webkit'
const engines: Record<EngineName, BrowserType> = { chromium, firefox, webkit }


export type Person = { key: string; email: string; name: string; role: string }

/** The accounts `seed-multi-user.sh` makes (same public test password as the other end-to-end accounts). */
export const people = {
  owner: { key: 'owner', email: process.env.E2E_MU_OWNER ?? 'mu-owner@example.test', name: 'MU Owner', role: 'workspace OWNER, Project admin' },
  admin: { key: 'admin', email: process.env.E2E_MU_ADMIN ?? 'mu-admin@example.test', name: 'MU Admin', role: 'workspace ADMIN, not in the Project' },
  member: { key: 'member', email: process.env.E2E_MU_MEMBER ?? 'mu-member@example.test', name: 'MU Member', role: 'workspace MEMBER, Project editor, in the private channel' },
  member2: { key: 'member2', email: process.env.E2E_MU_MEMBER2 ?? 'mu-member2@example.test', name: 'MU Member Two', role: 'workspace MEMBER, Project editor, not in the private channel' },
  outsider: { key: 'outsider', email: process.env.E2E_MU_OUTSIDER ?? 'mu-outsider@example.test', name: 'MU Outsider', role: 'workspace MEMBER, not in the Project' },
} satisfies Record<string, Person>

type LogEntry = { at: string; who: string; kind: string; text: string }

export class Journal {
  readonly entries: LogEntry[] = []
  constructor(readonly file: string) {
    mkdirSync(dirname(file), { recursive: true })
  }

  write(who: string, kind: string, text: string) {
    const entry = { at: new Date().toISOString(), who, kind, text }
    this.entries.push(entry)
    appendFileSync(this.file, `${JSON.stringify(entry)}\n`)
    console.log(`${entry.at} [${who}] ${kind}: ${text}`)
  }
}

export class Actor {
  readonly problems: string[] = []
  private static readonly always: RegExp[] = [
    /Service Worker registration blocked by Playwright/,
    /Viewport argument key "interactive-widget" not recognized/,
    /scroll-linked positioning effect/,
  ]
  private allowed: RegExp[] = [...Actor.always]
  private lastNavigation = 0

  constructor(
    readonly person: Person,
    readonly engine: EngineName,
    readonly context: BrowserContext,
    readonly page: Page,
    readonly journal: Journal,
    readonly baseURL: string,
  ) {
    this.watch(page)
  }

  get tag() {
    return `${this.person.key}@${this.engine}`
  }

  /** Problems that this step causes on purpose (a refused request, say): they are logged, not counted. */
  allow(pattern: RegExp) {
    this.allowed.push(pattern)
  }

  /** Ends a test: hands back what went wrong and forgets what that test allowed. */
  settle(): string[] {
    const problems = this.problems.splice(0)
    this.allowed = [...Actor.always]
    return problems
  }

  private note(kind: string, text: string) {
    // Browsers log a refused request to the console as well; it is as expected as the response it reports.
    const logged = /Failed to load resource: the server responded with a status of (\d{3})/.exec(text)
    const reviewed = this.allowed.some((pattern) => pattern.test(text) || (logged !== null && pattern.test(`${logged[1]} `)))
    this.journal.write(this.tag, reviewed ? `${kind} (expected)` : kind, text)
    if (!reviewed) this.problems.push(`${this.tag} ${kind}: ${text}`)
  }

  private watch(page: Page) {
    page.on('request', (request) => {
      if (request.isNavigationRequest() && request.frame() === page.mainFrame()) this.lastNavigation = Date.now()
    })
    page.on('console', (message) => {
      if (message.type() === 'error' || message.type() === 'warning') this.note(`console.${message.type()}`, message.text())
    })
    page.on('pageerror', (error) => {
      // WebKit reports a fetch that a navigation cut off as "... due to access control checks"; the app's query
      // handles the rejection. Only right after a navigation does it count as that.
      if (Date.now() - this.lastNavigation < 2_000 && /due to access control checks/.test(error.message)) {
        this.journal.write(this.tag, 'navigation cancellation', `page error: ${error.message}`)
      } else {
        this.note('page error', error.message)
      }
    })
    page.on('requestfailed', (request) => {
      const reason = request.failure()?.errorText ?? 'unknown'
      const navigating = Date.now() - this.lastNavigation < 2_000
      if (navigating && /cancel|ABORTED/i.test(reason)) {
        this.journal.write(this.tag, 'navigation cancellation', `${request.method()} ${request.url()} (${reason})`)
      } else {
        this.note('request failed', `${request.method()} ${request.url()} (${reason})`)
      }
    })
    page.on('response', async (response) => {
      if (response.status() < 400) return
      const body = await response.text().catch(() => '')
      this.note('http', `${response.status()} ${response.request().method()} ${response.url()} ${body.slice(0, 300)}`)
    })
    page.on('websocket', (socket) => {
      this.journal.write(this.tag, 'socket open', socket.url())
      socket.on('socketerror', (error) => {
        const navigating = Date.now() - this.lastNavigation < 2_000
        if (navigating) this.journal.write(this.tag, 'navigation socket close', `${socket.url()} ${error}`)
        else this.note('socket error', `${socket.url()} ${error}`)
      })
    })
  }

  /** One action of this person, logged before and after, with how long it took. */
  async act<T>(action: string, run: () => Promise<T>): Promise<T> {
    const started = Date.now()
    this.journal.write(this.tag, 'action', `${action} (start)`)
    try {
      const result = await run()
      this.journal.write(this.tag, 'action', `${action} (done in ${Date.now() - started} ms)`)
      return result
    } catch (error) {
      this.journal.write(this.tag, 'action failed', `${action}: ${String(error).slice(0, 500)}`)
      await this.page.screenshot({ path: this.journal.file.replace(/\.jsonl$/, `-${this.tag}-${Date.now()}.png`) }).catch(() => undefined)
      throw error
    }
  }

  /** The API as this person, through the browser context's own cookie jar (so it is the same session). */
  async api(method: string, path: string, body?: unknown): Promise<APIResponse> {
    const response = await this.context.request.fetch(`${this.baseURL}${path}`, {
      method,
      headers: { 'x-penfii-client': '1', 'idempotency-key': randomUUID(), origin: this.baseURL },
      data: body,
    })
    this.journal.write(this.tag, 'api', `${method} ${path} -> ${response.status()}`)
    return response
  }

  async json<T>(method: string, path: string, body?: unknown): Promise<T> {
    const response = await this.api(method, path, body)
    const text = await response.text()
    expect(response.status(), `${this.tag} ${method} ${path}: ${text}`).toBeLessThan(300)
    return (text ? JSON.parse(text) : undefined) as T
  }

  async logIn() {
    await this.act('log in through the login screen', async () => {
      await this.page.goto(`${this.baseURL}/login`)
      await this.page.getByLabel('Email').fill(this.person.email)
      await this.page.getByLabel('Password').fill(PASSWORD)
      await this.page.getByRole('button', { name: 'Log in' }).click()
      await expect(this.page).not.toHaveURL(/\/login/)
      await expect(this.page.getByRole('button', { name: this.person.name, exact: true })).toBeVisible()
    })
  }
}

/** Opens the engines once and hands out one fresh context (own cookies, own storage) per person. */
export class Stage {
  private readonly browsers = new Map<EngineName, Browser>()
  readonly actors: Actor[] = []

  constructor(readonly journal: Journal, readonly baseURL: string) {}

  async enter(person: Person, engine: EngineName): Promise<Actor> {
    let browser = this.browsers.get(engine)
    if (!browser) {
      browser = await engines[engine].launch()
      this.journal.write('stage', 'browser', `${engine} ${browser.version()}`)
      this.browsers.set(engine, browser)
    }
    const context = await browser.newContext({ locale: 'en-US', serviceWorkers: 'block', viewport: { width: 1360, height: 900 } })
    const page = await context.newPage()
    const actor = new Actor(person, engine, context, page, this.journal, this.baseURL)
    this.actors.push(actor)
    await actor.logIn()
    return actor
  }

  settle() {
    return this.actors.flatMap((actor) => actor.settle())
  }

  async close() {
    for (const actor of this.actors) await actor.context.close().catch(() => undefined)
    for (const browser of this.browsers.values()) await browser.close().catch(() => undefined)
  }
}

/**
 * Resolves when the page's Project socket has said `project.ready` (the server has subscribed it to the change feed),
 * so a test can start a change only once every watcher can hear about it. Call it before the navigation that opens the
 * socket.
 */
export function projectSocketReady(page: Page, timeout = 15_000): Promise<void> {
  return socketReady(page, '/v1/ws/projects/', 'project.ready', timeout)
}

/** Resolves when the page's user socket has said `user.ready`. Call it before the navigation that opens the socket. */
export function userSocketReady(page: Page, timeout = 15_000): Promise<void> {
  return socketReady(page, '/v1/ws/user', 'user.ready', timeout)
}

function socketReady(page: Page, path: string, ready: string, timeout: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      page.off('websocket', onSocket)
      reject(new Error(`the socket ${path} did not say ${ready}`))
    }, timeout)
    const onSocket = (socket: import('@playwright/test').WebSocket) => {
      if (!socket.url().includes(path)) return
      socket.on('framereceived', (frame) => {
        if (String(frame.payload).includes(`"${ready}"`)) {
          clearTimeout(timer)
          page.off('websocket', onSocket)
          resolve()
        }
      })
    }
    page.on('websocket', onSocket)
  })
}

/** Starts the actions of several people in the same tick and waits for all of them (each prepares its screen first). */
export async function together<T extends unknown[]>(...runs: { [K in keyof T]: () => Promise<T[K]> }): Promise<T> {
  return Promise.all(runs.map((run) => run())) as Promise<T>
}
