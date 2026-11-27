import { request as playwrightRequest, type Locator, type Page } from '@playwright/test'
import { expect, test } from '../support/test'
import { accountA } from '../support/accounts'
import { apiAs, logIn } from '../support/api'

const suggestion = (page: Page, name: string) => page.locator('.bn-suggestion-menu-item', { hasText: name }).first()

/**
 * Clicks a BlockNote menu item where it is on screen, as a person does. Playwright's WebKit click scrolls the
 * Ticket panel first even when the item is in view; the menu then follows its caret out of sight and hides.
 */
async function clickMenuItem(page: Page, item: Locator) {
  await expect(item).toBeVisible()
  const box = await item.boundingBox()
  expect(box).not.toBeNull()
  await page.mouse.click(box!.x + box!.width / 2, box!.y + box!.height / 2)
}

test.describe('FEA-104–107 channel and ticket editing', () => {
  test('keeps the Context Nav conversation menu visible and operable at the viewport edge', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    for (let index = 0; index < 12; index += 1) await api.createChannel(workspaceId, `float-${Date.now()}-${index}`)
    await logIn(page, accountA.email)
    await page.addInitScript((id) => {
      localStorage.setItem(`penfii.chat.sections.${id}`, JSON.stringify({ sections: Array.from({ length: 18 }, (_, index) => ({ id: `section-${index}`, name: `Section ${index}`, conversationIds: [] })) }))
    }, workspaceId)
    await page.goto(`/w/${workspaceId}/chat`)
    await page.locator('.workspace-nav').getByRole('button', { name: 'Show more' }).first().click()

    const nav = page.locator('.workspace-nav')
    const trigger = nav.getByRole('button', { name: 'Conversation menu' }).last()
    await trigger.scrollIntoViewIfNeeded()
    await trigger.click()
    const menu = page.getByRole('menu')
    await expect(menu).toBeVisible()
    await expect.poll(() => menu.evaluate((element) => {
      const rect = element.getBoundingClientRect()
      return rect.top >= 0 && rect.left >= 0 && rect.bottom <= window.innerHeight && rect.right <= window.innerWidth
    })).toBe(true)
    const beforeScroll = await nav.evaluate((element) => element.scrollTop)
    await nav.evaluate((element) => element.scrollBy(0, -80))
    await expect.poll(() => nav.evaluate((element) => element.scrollTop)).not.toBe(beforeScroll)
    await page.setViewportSize({ width: 1120, height: 640 })
    await expect.poll(() => menu.evaluate((element) => element.getBoundingClientRect().bottom <= window.innerHeight)).toBe(true)

    // The Favorite row (FEA-201) is the menu's last item; the last group is just above it.
    await page.keyboard.press('End')
    await expect(page.locator('[role="menuitemcheckbox"]:focus')).toContainText(/Favorites/)
    await page.keyboard.press('ArrowUp')
    await expect(page.locator('[role="menuitem"]:focus')).toContainText('Section 17')
    await page.keyboard.press('Escape')
    await expect(menu).toHaveCount(0)
    await expect(trigger).toBeFocused()
    await trigger.click()
    await page.mouse.click(600, 80)
    await expect(menu).toHaveCount(0)
  })

  // The seeded accounts are both workspace OWNERs, so this needs a plain MEMBER: E2E_EMAIL_MEMBER (same password).
  test('does not offer Channel Settings to a member who may not manage the channel', async ({ page, request }) => {
    const memberEmail = process.env.E2E_EMAIL_MEMBER
    test.skip(!memberEmail, 'set E2E_EMAIL_MEMBER to an account with the MEMBER role to run this check')
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `member-view-${Date.now()}`)
    await logIn(page, memberEmail!)
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}`)
    await expect(page.getByRole('heading', { name: `# ${channel.name}` })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Channel settings' })).toHaveCount(0)
    // Typing the settings address by hand opens a read-only panel: no management control is offered.
    await page.goto(`/w/${workspaceId}/c/${channel.conversationId}?settings=1`)
    await expect(page.getByRole('heading', { name: `# ${channel.name}` })).toBeVisible()
    const panel = page.getByRole('dialog', { name: 'Channel settings' })
    await expect(panel.getByText('Read only: you do not have permission to change the settings of this channel.')).toBeVisible()
    for (const control of ['Webhooks', 'Add people']) await expect(panel.getByRole('button', { name: control })).toHaveCount(0)
    await expect(panel.getByLabel('Visibility')).toHaveCount(0)
    await panel.locator('.channel-settings-head').getByRole('button', { name: 'Close' }).click()

    // A plain member of a private channel keeps the member list in the conversation header, without the settings.
    const memberRequest = await playwrightRequest.newContext({ baseURL: test.info().project.use.baseURL, ignoreHTTPSErrors: test.info().project.use.ignoreHTTPSErrors })
    const memberId = (await (await apiAs(memberRequest, memberEmail!)).me()).id
    await memberRequest.dispose()
    const privateChannel = await api.createChannel(workspaceId, `member-private-${Date.now()}`, 'PRIVATE')
    await api.addChannelMember(privateChannel.id, memberId)
    await page.goto(`/w/${workspaceId}/c/${privateChannel.conversationId}`)
    await expect(page.getByRole('heading', { name: `# ${privateChannel.name}` })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Channel settings' })).toHaveCount(0)
    await page.locator('.context-actions').getByRole('button', { name: 'Add people' }).click()
    const members = page.getByRole('dialog', { name: 'Add people to this channel' })
    await expect(members.getByText(accountA.name)).toBeVisible()
    await members.getByRole('button', { name: 'Close' }).click()
  })

  test('opens Channel Settings from Context Nav without losing the conversation', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const channel = await api.createChannel(workspaceId, `settings-${Date.now()}`)
    const privateChannel = await api.createChannel(workspaceId, `private-settings-${Date.now()}`, 'PRIVATE')
    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/chat`)

    const showMore = page.locator('.workspace-nav').getByRole('button', { name: 'Show more' }).first()
    await expect(showMore).toBeVisible()
    await showMore.click()
    const channelLink = page.locator(`.workspace-nav a.tree-link[href*="/c/${channel.conversationId}"]`)
    await expect(channelLink).toHaveCount(1)
    const channelRow = channelLink.locator('xpath=..')
    await channelRow.getByRole('button', { name: 'Channel settings' }).click()
    await expect(page).toHaveURL(new RegExp(`/w/${workspaceId}/c/${channel.conversationId}\\?settings=1`))
    const panel = page.getByRole('dialog', { name: 'Channel settings' })
    await expect(panel).toBeVisible()
    await expect(panel.getByText(`# ${channel.name}`, { exact: true })).toBeVisible()
    const webhooksButton = panel.getByRole('button', { name: 'Webhooks' })
    await expect(webhooksButton).toBeVisible()
    await webhooksButton.click()
    const webhooksDialog = page.getByRole('dialog', { name: 'Incoming webhooks' })
    await expect(webhooksDialog).toBeVisible()
    await expect(webhooksDialog.getByText('This channel has no webhooks.')).toBeVisible()
    await webhooksDialog.getByRole('button', { name: 'Done' }).click()
    const visibility = panel.getByLabel('Visibility')
    await visibility.selectOption('PRIVATE')
    const confirmVisibility = page.getByRole('dialog', { name: 'Visibility' })
    await expect(confirmVisibility).toBeVisible()
    await confirmVisibility.getByRole('button', { name: 'Cancel' }).click()
    await expect(page.getByRole('heading', { name: `# ${channel.name}` })).toBeVisible()
    await panel.getByRole('button', { name: 'Close' }).click()
    await expect(page).not.toHaveURL(/settings=1/)
    await expect(page.getByRole('heading', { name: `# ${channel.name}` })).toBeVisible()

    await page.goto(`/w/${workspaceId}/chat`)
    const privateShowMore = page.locator('.workspace-nav').getByRole('button', { name: 'Show more' }).first()
    await expect(privateShowMore).toBeVisible()
    await privateShowMore.click()
    const privateLink = page.locator(`.workspace-nav a.tree-link[href*="/c/${privateChannel.conversationId}"]`)
    await expect(privateLink).toHaveCount(1)
    const privateRow = privateLink.locator('xpath=..')
    await privateRow.getByRole('button', { name: 'Channel settings' }).click()
    const privatePanel = page.getByRole('dialog', { name: 'Channel settings' })
    await expect(privatePanel).toBeVisible()
    await privatePanel.getByRole('button', { name: 'Add people' }).click()
    const membersDialog = page.getByRole('dialog', { name: 'Add people to this channel' })
    await expect(membersDialog.getByText('E2E Alpha')).toBeVisible()
    await membersDialog.getByRole('button', { name: 'Close' }).click()
    await privatePanel.locator('.channel-settings-head').getByRole('button', { name: 'Close' }).click()
    await expect(page).toHaveURL(`/w/${workspaceId}/c/${privateChannel.conversationId}`)
  })

  test('edits a legacy ticket description through the Docs BlockNote model and stores v1 content', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `Ticket rich text ${Date.now()}`)
    const legacy = { type: 'doc', content: [
      ...Array.from({ length: 12 }, (_, index) => ({ type: 'paragraph', content: [{ type: 'text', text: `Legacy description line ${index + 1}` }] })),
      { type: 'bulletList', content: [{ type: 'listItem', content: [
        { type: 'paragraph', content: [{ type: 'text', text: 'Legacy parent item' }] },
        { type: 'bulletList', content: [{ type: 'listItem', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Legacy nested item' }] }] }] },
      ] }] },
      { type: 'blockquote', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Legacy quote', marks: [{ type: 'bold' }] }] }] },
      { type: 'paragraph', content: [{ type: 'text', text: 'Legacy link', marks: [{ type: 'link', attrs: { href: 'https://example.org' } }] }] },
      { type: 'legacyWidget', attrs: { label: 'preserve safely' }, content: [] },
    ] }
    const ticket = await api.createTicket(project.id, 'Legacy description', legacy)

    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    expect(await page.evaluate(() => isSecureContext && navigator.onLine), 'writes require a secure, online browser context').toBe(true)
    const panel = page.getByRole('complementary', { name: 'Ticket detail' })
    const read = page.locator('.ticket-description-read')
    await expect(read).toContainText('Legacy description line 1')
    await expect(read).toContainText('Unsupported legacy node')
    await expect(read).toHaveAttribute('aria-expanded', 'false')
    await read.click()
    await expect(read).toHaveAttribute('aria-expanded', 'true')
    await read.click()
    await expect(read).toHaveAttribute('aria-expanded', 'false')
    await read.click()
    await expect(read).toHaveAttribute('aria-expanded', 'true')
    // The description's editor; the comment box under the panel is an editor too.
    await expect(panel.locator('.ticket-description-blocknote [contenteditable="true"]')).toHaveCount(0)

    await panel.getByRole('button', { name: 'Edit description' }).click()
    const editor = panel.locator('.ticket-description-blocknote [contenteditable="true"]').first()
    await expect(editor).toBeVisible()
    await editor.click()
    await page.keyboard.press('Control+End')
    await page.keyboard.press('Enter')
    await page.keyboard.press('Control+B')
    await page.keyboard.type('Bold paragraph')
    await page.keyboard.press('Control+B')
    await page.keyboard.type(' ')
    await page.keyboard.press('Control+I')
    await page.keyboard.type('italic text')
    await page.keyboard.press('Control+I')

    for (const [command, title, text] of [
      ['heading', 'Heading 1', 'Rich heading'],
      ['bullet list', 'Bullet List', 'Bullet item'],
      ['check list', 'Check List', 'Checklist item'],
      ['quote', 'Quote', 'Quoted text'],
    ]) {
      await page.keyboard.press('Enter')
      await page.keyboard.type(`/${command}`)
      const option = page.getByText(title, { exact: true }).last()
      await expect(option).toBeVisible()
      await option.click()
      await page.keyboard.type(text)
    }
    await page.keyboard.press('Enter')
    await page.keyboard.type('https://example.com ')
    await page.keyboard.press('Enter')
    await page.keyboard.type('/code block')
    const codeBlockOption = page.getByText('Code Block', { exact: true }).last()
    await expect(codeBlockOption).toBeVisible()
    await codeBlockOption.click()
    await page.keyboard.type('const answer = 42')
    await panel.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(panel.locator('.ticket-description-blocknote [contenteditable="true"]')).toHaveCount(0)
    await expect(panel.locator('.ticket-description-read')).toContainText('Bold paragraph')
    await expect(panel.locator('.ticket-description-read')).toContainText('Rich heading')
    await expect(panel.locator('.ticket-description-read')).toContainText('Checklist item')
    await expect(panel.locator('.ticket-description-read')).toContainText('Quoted text')
    await expect(panel.locator('.ticket-description-read')).toContainText('const answer = 42')
    await expect(panel.locator('.ticket-description-read a[href="https://example.com"]')).toBeVisible()
    const stored = await api.ticket(ticket.id)
    expect(stored.description).toMatchObject({ schema: 'penfii.doc', version: 1 })
    const saved = JSON.stringify(stored.description)
    // The converted legacy content went through the server's document rules with the edit.
    for (const legacyText of ['Legacy nested item', 'Legacy quote', 'https://example.org', 'preserve safely']) expect(saved).toContain(legacyText)
    for (const blockType of ['"type":"heading"', '"type":"bulletListItem"', '"type":"checkListItem"', '"type":"quote"', '"type":"codeBlock"', '"bold":true', '"italic":true', '"type":"link"']) expect(saved).toContain(blockType)
    await panel.getByRole('button', { name: 'Close' }).click()
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    await expect(page.locator('.ticket-description-read')).toContainText('Bold paragraph')
    await page.getByRole('button', { name: 'Edit description' }).click()
    const reopenedEditor = page.locator('.ticket-description-blocknote [contenteditable="true"]').first()
    await reopenedEditor.click()
    await page.keyboard.press('Control+A')
    await page.keyboard.type('Canceled edit should not persist')
    await page.getByRole('button', { name: 'Cancel', exact: true }).click()
    await expect(page.locator('.ticket-description-read')).toContainText('Bold paragraph')
    expect(JSON.stringify((await api.ticket(ticket.id)).description)).toContain('Bold paragraph')
    expect(JSON.stringify((await api.ticket(ticket.id)).description)).not.toContain('Canceled edit should not persist')
  })

  test('inserts a mention and a Ticket reference, and Escape in a menu or the picker keeps the draft', async ({ page, request }) => {
    const api = await apiAs(request, accountA.email)
    const workspaceId = await api.workspaceId()
    const project = await api.createProject(workspaceId, `Ticket references ${Date.now()}`)
    const target = await api.createTicket(project.id, `Reference target ${Date.now()}`)
    const ticket = await api.createTicket(project.id, 'Mentions and references')

    await logIn(page, accountA.email)
    await page.goto(`/w/${workspaceId}/projects/${project.id}/work?panel=ticket:${ticket.id}`)
    const panel = page.getByRole('complementary', { name: 'Ticket detail' })
    await panel.getByRole('button', { name: 'Edit description' }).click()
    const editor = panel.locator('.ticket-description-blocknote [contenteditable="true"]').first()
    await expect(editor).toBeVisible()
    await editor.click()
    await page.keyboard.type('Ask ')
    await page.keyboard.type('@E2E')
    // The directory search leaves the signed-in person out: mention whoever else it offers first.
    const person = page.locator('.bn-suggestion-menu-item').first()
    await expect(person).toBeVisible()
    const personName = (await person.locator('.bn-mt-suggestion-menu-item-title').textContent())?.trim() ?? ''
    expect(personName).not.toBe('')
    await clickMenuItem(page, person)
    await expect(editor.locator('.doc-chip-user')).toHaveCount(1)

    // Escape closes the slash menu, not the edit.
    await page.keyboard.type('/')
    await expect(suggestion(page, 'Reference a Ticket')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.locator('.bn-suggestion-menu')).toHaveCount(0)
    await expect(editor).toBeVisible()

    // Escape closes the picker, not the edit; what was typed is still there.
    await page.keyboard.type('/Reference a Ticket')
    await clickMenuItem(page, suggestion(page, 'Reference a Ticket'))
    const picker = page.getByRole('dialog', { name: 'Choose a Ticket' })
    await expect(picker).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(picker).toHaveCount(0)
    await expect(editor).toBeVisible()
    await expect(editor.locator('.doc-chip-user')).toHaveCount(1)

    await page.keyboard.type('/Reference a Ticket')
    await clickMenuItem(page, suggestion(page, 'Reference a Ticket'))
    await expect(picker).toBeVisible()
    await picker.getByRole('searchbox').fill(target.key)
    await picker.getByRole('button', { name: new RegExp(target.key) }).click()
    await expect(picker).toHaveCount(0)
    await expect(editor.locator('.doc-chip-ticket')).toContainText('Reference target')

    await panel.getByRole('button', { name: 'Save', exact: true }).click()
    const read = panel.locator('.ticket-description-read')
    await expect(panel.locator('.ticket-description-blocknote [contenteditable="true"]')).toHaveCount(0)
    await expect(read.locator('.doc-chip-user')).toContainText(personName)
    await expect(read.locator('.doc-chip-ticket')).toContainText('Reference target')
    const saved = JSON.stringify((await api.ticket(ticket.id)).description)
    expect(saved).toContain('"type":"mention"')
    expect(saved).toContain(`"id":"${target.id}"`)

    await page.reload()
    await expect(read.locator('.doc-chip-user')).toContainText(personName)
    await read.locator('.doc-chip-ticket').click()
    await expect(page).toHaveURL(new RegExp(`panel=ticket:${target.id}`))
  })
})
