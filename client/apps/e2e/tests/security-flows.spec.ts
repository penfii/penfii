import { createHmac } from 'node:crypto'
import { expect, type Browser, type Page } from '@playwright/test'
import { PASSWORD } from '../support/accounts'
import { apiAs } from '../support/api'
import { accountA } from '../support/accounts'
import { test } from '../support/test'

// FEA-140 (e-mail change) and FEA-150 (two-step verification) as a person does them in the browser. They use their
// own accounts (`seed-security.sh`), so a failed run never locks the shared test accounts. Needs Mailpit for the mail.
const mailpit = process.env.E2E_MAILPIT
const MFA_EMAIL = process.env.E2E_EMAIL_MFA ?? 'e2e-mfa@example.test'
const CHANGE_EMAIL = process.env.E2E_EMAIL_CHANGE ?? 'e2e-chg@example.test'

/** RFC 6238 (SHA-1, 6 digits, 30 s) from the base32 key the setup screen shows. */
function totp(secret: string, offsetSteps = 0): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
  let bits = ''
  for (const ch of secret.replace(/[\s=]/g, '').toUpperCase()) bits += alphabet.indexOf(ch).toString(2).padStart(5, '0')
  const bytes = Buffer.from(bits.match(/.{8}/g)!.map((byte) => parseInt(byte, 2)))
  const counter = Buffer.alloc(8)
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30_000) + offsetSteps))
  const digest = createHmac('sha1', bytes).update(counter).digest()
  const offset = digest[digest.length - 1] & 0x0f
  const value = ((digest[offset] & 0x7f) << 24) | (digest[offset + 1] << 16) | (digest[offset + 2] << 8) | digest[offset + 3]
  return String(value % 1_000_000).padStart(6, '0')
}

async function stranger(browser: Browser, watch: (page: Page) => void) {
  const context = await browser.newContext({ locale: 'en-US', serviceWorkers: 'block' })
  const page = await context.newPage()
  watch(page)
  return { context, page }
}

async function passwordLogin(page: Page, email: string) {
  await page.goto('/login')
  await page.getByLabel('Email').fill(email)
  await page.getByLabel('Password').fill(PASSWORD)
  await page.getByRole('button', { name: 'Log in' }).click()
}

async function confirmPasswordIfAsked(page: Page) {
  const dialog = page.getByRole('dialog', { name: 'Confirm it is you' })
  if (await dialog.isVisible({ timeout: 1500 }).catch(() => false)) {
    await dialog.getByLabel('Password').fill(PASSWORD)
    await dialog.getByRole('button', { name: 'Confirm' }).click()
  }
}

/** Waits for the next 30-second step so a code that was just spent is not reused. */
async function nextStep() {
  const wait = 30_000 - (Date.now() % 30_000) + 500
  await new Promise((resolve) => setTimeout(resolve, wait))
}

test.describe('two-step verification (FEA-150)', () => {
  test('set up with a code, sign in with the code and with a recovery code, wrong codes are refused, turn off', async ({ browser, watchPage, allowProblem }) => {
    test.setTimeout(240_000)
    allowProblem(/401 (GET|POST) .*\/v1\/(me|session\/login)/)
    allowProblem(/failed to load resource.*(401|403|409)/i)
    const { context, page } = await stranger(browser, watchPage)
    await passwordLogin(page, MFA_EMAIL)
    await expect(page).not.toHaveURL(/\/login/)

    await page.goto('/account')
    await page.getByRole('button', { name: 'Set up' }).click()
    await confirmPasswordIfAsked(page)
    if (await page.getByRole('button', { name: 'Set up' }).isVisible().catch(() => false)) await page.getByRole('button', { name: 'Set up' }).click()

    // The QR code is drawn in the browser; the key is shown for typing too.
    await expect(page.getByRole('img', { name: 'QR code for your authenticator app' })).toBeVisible()
    const secretText = await page.locator('code, kbd, samp, [data-testid="mfa-secret"]').filter({ hasText: /^[A-Z2-7 ]{16,}$/ }).first().innerText()
    expect(secretText.replace(/ /g, '')).toMatch(/^[A-Z2-7]{32}$/)

    // A wrong code does not turn it on.
    await page.getByLabel('Code from the app').fill('000000')
    await page.getByRole('button', { name: 'Turn on' }).click()
    await expect(page.getByText('That code is not correct')).toBeVisible()

    await page.getByLabel('Code from the app').fill(totp(secretText))
    await page.getByRole('button', { name: 'Turn on' }).click()
    await expect(page.getByRole('heading', { name: 'Your recovery codes' }).or(page.getByText('Your recovery codes'))).toBeVisible()
    const codesText = await page.locator('body').innerText()
    const codes = codesText.match(/\b[0-9a-hjkmnp-tv-z]{4}-[0-9a-hjkmnp-tv-z]{4}-[0-9a-hjkmnp-tv-z]{4}\b/g) ?? []
    expect(new Set(codes).size).toBe(10)
    await page.getByRole('button', { name: 'I have saved them' }).click()
    await expect(page.getByText('10 recovery codes left')).toBeVisible()
    await context.close()

    // A second browser: the password alone does not get in; it asks for the code.
    const second = await stranger(browser, watchPage)
    await second.page.goto('/login')
    await second.page.getByLabel('Email').fill(MFA_EMAIL)
    await second.page.getByLabel('Password').fill(PASSWORD)
    await second.page.getByRole('button', { name: 'Log in' }).click()
    await expect(second.page.getByText('Two-step verification is on for this account', { exact: false })).toBeVisible()
    await expect(second.page).toHaveURL(/\/login/)
    const cookiesBefore = (await second.context.cookies()).filter((cookie) => cookie.name === 'penfii_session')
    expect(cookiesBefore, 'no session before the code').toHaveLength(0)

    await second.page.getByLabel('Authenticator or recovery code').fill('123456')
    await second.page.getByRole('button', { name: 'Log in' }).click()
    await expect(second.page.getByText('The code is incorrect or was already used')).toBeVisible()

    // The setup code was spent; the next step's code is right.
    await nextStep()
    await second.page.getByLabel('Authenticator or recovery code').fill(totp(secretText))
    await second.page.getByRole('button', { name: 'Log in' }).click()
    await expect(second.page).not.toHaveURL(/\/login/)
    await second.context.close()

    // A recovery code signs in once; the same code again is refused.
    const third = await stranger(browser, watchPage)
    await third.page.goto('/login')
    await third.page.getByLabel('Email').fill(MFA_EMAIL)
    await third.page.getByLabel('Password').fill(PASSWORD)
    await third.page.getByRole('button', { name: 'Log in' }).click()
    await third.page.getByLabel('Authenticator or recovery code').fill(codes[0]!)
    await third.page.getByRole('button', { name: 'Log in' }).click()
    await expect(third.page).not.toHaveURL(/\/login/)
    await third.context.close()

    const fourth = await stranger(browser, watchPage)
    await fourth.page.goto('/login')
    await fourth.page.getByLabel('Email').fill(MFA_EMAIL)
    await fourth.page.getByLabel('Password').fill(PASSWORD)
    await fourth.page.getByRole('button', { name: 'Log in' }).click()
    await fourth.page.getByLabel('Authenticator or recovery code').fill(codes[0]!)
    await fourth.page.getByRole('button', { name: 'Log in' }).click()
    await expect(fourth.page.getByText('The code is incorrect or was already used')).toBeVisible()
    await expect(fourth.page).toHaveURL(/\/login/)

    // Turn it off from a signed-in browser with another recovery code; the account signs in with the password alone again.
    await fourth.page.getByLabel('Authenticator or recovery code').fill(codes[1]!)
    await fourth.page.getByRole('button', { name: 'Log in' }).click()
    await expect(fourth.page).not.toHaveURL(/\/login/)
    await fourth.page.goto('/account')
    await expect(fourth.page.getByText('8 recovery codes left')).toBeVisible()
    await fourth.page.getByRole('button', { name: 'Turn off' }).click()
    await confirmPasswordIfAsked(fourth.page)
    if (!(await fourth.page.getByLabel('Authenticator or recovery code').isVisible().catch(() => false))) {
      await fourth.page.getByRole('button', { name: 'Turn off' }).click()
    }
    await fourth.page.getByLabel('Authenticator or recovery code').fill(codes[2]!)
    await fourth.page.getByRole('button', { name: 'Turn off' }).last().click()
    await expect(fourth.page.getByRole('button', { name: 'Set up' })).toBeVisible()
    await fourth.context.close()

    const last = await stranger(browser, watchPage)
    await passwordLogin(last.page, MFA_EMAIL)
    await expect(last.page).not.toHaveURL(/\/login/)
    await last.context.close()
  })
})

test.describe('e-mail change (FEA-140)', () => {
  // The server allows 5 change requests an hour per account and 3 an hour per target address; one run makes two requests
  // (there and back), so a third run within the hour is answered 429 on purpose. Wait, or clear `auth_throttle` of a throwaway DB.
  test.skip(!mailpit, 'set E2E_MAILPIT to the Mailpit the stack sends to')

  async function mailedLink(address: string, since: number): Promise<string> {
    let link = ''
    await expect
      .poll(async () => {
        const search = await fetch(`${mailpit}/api/v1/search?query=${encodeURIComponent(`to:"${address}"`)}`)
        const found = (await search.json()) as { messages: Array<{ ID: string; Created: string }> }
        for (const message of found.messages) {
          if (new Date(message.Created).getTime() < since) continue
          const full = (await (await fetch(`${mailpit}/api/v1/message/${message.ID}`)).json()) as { Text: string }
          const match = full.Text.match(/https?:\/\/\S+\/confirm-email#\S+/)
          if (match) {
            link = match[0]
            return true
          }
        }
        return false
      }, { timeout: 20_000 })
      .toBe(true)
    return link
  }

  test('the address changes only after the link in the new address is opened, and the old address is told', async ({ browser, watchPage, allowProblem }) => {
    allowProblem(/401 (GET|POST) .*\/v1\/(me|session\/login)/)
    allowProblem(/failed to load resource.*(401|409)/i)
    // The second use of a single-use link is refused with 409 on purpose.
    allowProblem(/409 POST .*\/v1\/auth\/email-change\/confirm/)
    const stamp = Date.now().toString(36)
    const newEmail = `e2e-chg-${stamp}@example.test`
    const { context, page } = await stranger(browser, watchPage)
    await passwordLogin(page, CHANGE_EMAIL)
    await expect(page).not.toHaveURL(/\/login/)
    await page.goto('/account')

    const since = Date.now() - 2000
    await page.getByLabel('New e-mail address').fill(newEmail)
    await page.getByRole('button', { name: 'Send confirmation link' }).click()
    await confirmPasswordIfAsked(page)
    if (await page.getByRole('button', { name: 'Send confirmation link' }).isEnabled().catch(() => false)) {
      await page.getByRole('button', { name: 'Send confirmation link' }).click()
    }
    await expect(page.getByText(new RegExp(`(A confirmation link was sent to|Waiting for you to open the link sent to) ${newEmail}`))).toBeVisible()

    // Waiting changes nothing: the old address still signs in; the new one does not exist yet.
    const probe = await stranger(browser, watchPage)
    await passwordLogin(probe.page, newEmail)
    await expect(probe.page).toHaveURL(/\/login/)
    await probe.context.close()

    // Another person opens the link in a browser that is signed in as nobody: the link is the proof.
    const link = await mailedLink(newEmail, since)
    const opener = await stranger(browser, watchPage)
    await opener.page.goto(link)
    await expect(opener.page.getByText(/confirmed|changed|now/i).first()).toBeVisible()
    // The link works once.
    const again = await opener.context.newPage()
    watchPage(again)
    await again.goto(link)
    await expect(again.getByText(/not valid|invalid|expired|no longer/i).first()).toBeVisible()
    await opener.context.close()

    // The new address signs in now; the old one no longer does.
    const fresh = await stranger(browser, watchPage)
    await passwordLogin(fresh.page, newEmail)
    await expect(fresh.page).not.toHaveURL(/\/login/)
    const old = await stranger(browser, watchPage)
    await passwordLogin(old.page, CHANGE_EMAIL)
    await expect(old.page).toHaveURL(/\/login/)
    await old.context.close()

    // The old address was told.
    await expect
      .poll(async () => {
        const search = await fetch(`${mailpit}/api/v1/search?query=${encodeURIComponent(`to:"${CHANGE_EMAIL}"`)}`)
        // The notice is in the instance's default language (the account has no locale of its own).
        const found = (await search.json()) as { messages: Array<{ Subject: string }> }
        return found.messages.filter((message) => /e-mail address of your Penfii account was changed|이메일 주소가 변경되었습니다/.test(message.Subject)).length
      }, { timeout: 15_000 })
      .toBeGreaterThan(0)

    // Put the account back, so the test can run again.
    const backSince = Date.now() - 2000
    await fresh.page.goto('/account')
    await fresh.page.getByLabel('New e-mail address').fill(CHANGE_EMAIL)
    await fresh.page.getByRole('button', { name: 'Send confirmation link' }).click()
    await confirmPasswordIfAsked(fresh.page)
    if (await fresh.page.getByRole('button', { name: 'Send confirmation link' }).isEnabled().catch(() => false)) {
      await fresh.page.getByRole('button', { name: 'Send confirmation link' }).click()
    }
    const backLink = await mailedLink(CHANGE_EMAIL, backSince)
    const closer = await stranger(browser, watchPage)
    await closer.page.goto(backLink)
    await expect(closer.page.getByText(/confirmed|changed|now/i).first()).toBeVisible()
    await closer.context.close()
    await fresh.context.close()
    await context.close()
  })
})

// Removing a person from the Workspace and inviting them again (the v0.3.1 -> next hardening of removal and rejoin): a
// removed member loses the Workspace at once, and coming back through a new invitation gives the invited role only, not
// what they held before.
test.describe('removed member and rejoin', () => {
  test.skip(!mailpit, 'set E2E_MAILPIT to the Mailpit the stack sends to')

  test('the owner removes a member in the browser; the member is shut out at once; a new invitation brings them back without the old private access', async ({ browser, request, watchPage, allowProblem }) => {
    test.setTimeout(120_000)
    allowProblem(/40[134] (GET|POST|PUT|PATCH|DELETE) /)
    allowProblem(/failed to load resource.*(401|403|404)/i)
    const member = process.env.E2E_EMAIL_REJOIN ?? 'e2e-mfa@example.test'
    const owner = await apiAs(request, accountA.email)
    const workspaceId = await owner.workspaceId()
    const channel = await owner.createChannel(workspaceId, `rejoin-${Date.now().toString(36)}`, 'PRIVATE')

    const memberSide = await stranger(browser, watchPage)
    await passwordLogin(memberSide.page, member)
    await expect(memberSide.page).not.toHaveURL(/\/login/)
    const memberId = (await (await memberSide.context.request.get('/v1/me')).json()).id as string
    await owner.addChannelMember(channel.id, memberId)
    const read = () => memberSide.context.request.get(`/v1/conversations/${channel.conversationId}/messages`)
    expect((await read()).status(), 'member reads the private channel before removal').toBe(200)

    const ownerSide = await stranger(browser, watchPage)
    await passwordLogin(ownerSide.page, accountA.email)
    await expect(ownerSide.page).not.toHaveURL(/\/login/)
    await ownerSide.page.goto(`/w/${workspaceId}/settings/admin/members`)
    await ownerSide.page.getByRole('searchbox', { name: 'Search by name, e-mail or team' }).fill(member)
    await ownerSide.page.getByRole('button', { name: new RegExp(`Actions for .*${member}`) }).click()
    await ownerSide.page.getByRole('menuitem', { name: 'Remove' }).click()
    const confirm = ownerSide.page.getByRole('alertdialog').or(ownerSide.page.getByRole('dialog').filter({ hasText: /Remove/ })).last()
    await confirm.getByRole('button', { name: /^Remove/ }).click()
    // A removed person stays listed, marked Removed.
    await expect(ownerSide.page.getByRole('row').filter({ hasText: member })).toContainText('Removed')

    // At once: no read of the channel, no Workspace pages (the session itself lives on, it is the same account).
    expect([403, 404], 'removed member can not read the private channel').toContain((await read()).status())
    const space = await memberSide.context.request.get(`/v1/workspaces/${workspaceId}/space`)
    expect([403, 404], 'removed member can not open the Workspace').toContain(space.status())

    // Invite again by e-mail; the person, signed in, accepts from the mailed link.
    const since = Date.now() - 2000
    await ownerSide.page.goto(`/w/${workspaceId}/settings/admin/invitations`)
    await ownerSide.page.getByRole('button', { name: 'Invite member' }).click()
    const dialog = ownerSide.page.getByRole('dialog', { name: 'Invite member' })
    await dialog.getByRole('textbox', { name: 'Email' }).fill(member)
    await dialog.getByRole('button', { name: 'Invite member' }).click()
    await expect(dialog.getByRole('status')).toContainText(/invitation/i)
    let link = ''
    await expect
      .poll(async () => {
        const found = (await (await fetch(`${mailpit}/api/v1/search?query=${encodeURIComponent(`to:"${member}"`)}`)).json()) as { messages: Array<{ ID: string; Created: string }> }
        for (const message of found.messages) {
          if (new Date(message.Created).getTime() < since) continue
          const full = (await (await fetch(`${mailpit}/api/v1/message/${message.ID}`)).json()) as { Text: string }
          const match = full.Text.match(/https?:\/\/\S+\/invite\/\S+/)
          if (match) {
            link = match[0]
            return true
          }
        }
        return false
      }, { timeout: 20_000 })
      .toBe(true)
    await memberSide.page.goto(new URL(link).pathname)
    await memberSide.page.getByRole('button', { name: /Join|Accept/ }).click()
    await expect(memberSide.page.getByText(/You joined/)).toBeVisible()

    // Back in the Workspace, but not in the private channel, and with the invited role, not a higher one.
    const back = await memberSide.context.request.get(`/v1/workspaces/${workspaceId}/space`)
    expect(back.status(), 'rejoined member opens the Workspace').toBe(200)
    expect([403, 404], 'rejoining does not bring the old private channel back').toContain((await read()).status())
    const admin = await memberSide.context.request.get(`/v1/workspaces/${workspaceId}/members`)
    expect([403, 404], 'a rejoined MEMBER has no admin member list').toContain(admin.status())

    await ownerSide.context.close()
    await memberSide.context.close()
  })
})

test.describe('password change and signed-in devices', () => {
  test('changing the password in the browser ends the other devices at once and keeps this one; revoking a device from the list ends it', async ({ browser, watchPage, allowProblem }) => {
    allowProblem(/401 (GET|POST) /)
    allowProblem(/failed to load resource.*401/i)
    // Its own account, so a failure never leaves a shared one with another password.
    const email = process.env.E2E_EMAIL_PASSWORD ?? 'e2e-pwd@example.test'
    const changed = `Changed-${Date.now().toString(36)}-pass-2026-long`
    const here = await stranger(browser, watchPage)
    const other = await stranger(browser, watchPage)
    await passwordLogin(here.page, email)
    await expect(here.page).not.toHaveURL(/\/login/)
    await passwordLogin(other.page, email)
    await expect(other.page).not.toHaveURL(/\/login/)
    expect((await other.context.request.get('/v1/me')).status()).toBe(200)

    await here.page.goto('/account')
    await here.page.getByLabel('Current password', { exact: false }).first().fill(PASSWORD)
    await here.page.getByLabel('New password', { exact: false }).first().fill(changed)
    await here.page.getByRole('button', { name: 'Save' }).click()
    await expect(here.page.getByText('Password changed.', { exact: false })).toBeVisible()
    expect((await here.context.request.get('/v1/me')).status(), 'this device stays signed in').toBe(200)
    expect((await other.context.request.get('/v1/me')).status(), 'the other device is signed out').toBe(401)

    // The old password is refused, the new one works.
    const old = await stranger(browser, watchPage)
    await passwordLogin(old.page, email)
    await expect(old.page).toHaveURL(/\/login/)
    await old.context.close()
    const fresh = await stranger(browser, watchPage)
    await fresh.page.goto('/login')
    await fresh.page.getByLabel('Email').fill(email)
    await fresh.page.getByLabel('Password').fill(changed)
    await fresh.page.getByRole('button', { name: 'Log in' }).click()
    await expect(fresh.page).not.toHaveURL(/\/login/)

    // The device list shows both sessions; revoking the other one ends it, this one stays.
    await here.page.goto('/account')
    await expect(here.page.getByText('This device')).toBeVisible()
    const revoke = here.page.getByRole('button', { name: 'Sign out', exact: true })
    expect((await fresh.context.request.get('/v1/me')).status()).toBe(200)
    await expect(revoke).toHaveCount(1)
    await revoke.click()
    await expect.poll(async () => (await fresh.context.request.get('/v1/me')).status(), { timeout: 10_000 }).toBe(401)
    expect((await here.context.request.get('/v1/me')).status(), 'this device is not revoked by revoking another').toBe(200)

    // Put the password back.
    await here.page.goto('/account')
    await here.page.getByLabel('Current password', { exact: false }).first().fill(changed)
    await here.page.getByLabel('New password', { exact: false }).first().fill(PASSWORD)
    await here.page.getByRole('button', { name: 'Save' }).click()
    await expect(here.page.getByText('Password changed.', { exact: false })).toBeVisible()
    for (const side of [here, other, fresh]) await side.context.close()
  })
})
