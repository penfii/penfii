import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import type { RichTextDoc, TicketComment } from '@penfii/contracts'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  me: vi.fn(),
  getTicketComments: vi.fn(),
  createTicketComment: vi.fn(),
  editTicketComment: vi.fn(),
  deleteTicketComment: vi.fn(),
  getDocComments: vi.fn(),
  createDocComment: vi.fn(),
  editDocComment: vi.fn(),
  deleteDocComment: vi.fn(),
  searchDirectory: vi.fn(),
  resolveReferences: vi.fn(),
  createFile: vi.fn(),
  deleteFile: vi.fn(),
}))
vi.mock('../../api', () => ({ api }))

import { CommentComposer, CommentList } from '../work/CommentsTab'
import { DocComments } from '../docs/DocComments'
import { renderWithProviders } from '../../test/renderWithProviders'
import { editorOf, replaceTextIn, typeInto } from '../../test/richEditor'

const text = (value: string, marks?: unknown[]) => ({ type: 'text', text: value, ...(marks ? { marks } : {}) })
const para = (...content: unknown[]) => ({ type: 'paragraph', content })
const mention = (occurrenceId: string, userId: string) => ({ type: 'mention', attrs: { occurrenceId, kind: 'USER', userId } })

/** A body with every kind of thing the editor carries, so that changing one line shows whether the rest survived. */
const complex = {
  type: 'doc',
  content: [
    { type: 'heading', attrs: { level: 3 }, content: [text('Plan')] },
    para(text('first line'), text(' bold ', [{ type: 'bold' }]), text('and'), text(' linked', [{ type: 'link', attrs: { href: 'https://example.com/spec' } }]), text(' code', [{ type: 'code' }])),
    { type: 'orderedList', attrs: { start: 3 }, content: [
      { type: 'listItem', content: [para(text('three')), { type: 'bulletList', content: [{ type: 'listItem', content: [para(text('nested'))] }] }] },
      { type: 'listItem', content: [para(text('four'))] },
    ] },
    { type: 'blockquote', content: [para(text('quoted'))] },
    { type: 'codeBlock', attrs: { language: 'rust' }, content: [text('fn main() {}')] },
  ],
} as unknown as RichTextDoc

const ticketComment = (id: string, body: unknown, rights: Partial<TicketComment> = {}): TicketComment => ({
  id, ticketId: 't1', authorId: 'u1', authorName: 'Alice', body, createdAt: '2026-10-29T00:00:00Z', editedAt: null, attachments: [], canEdit: true, canDelete: true, ...rights,
}) as TicketComment

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.me.mockResolvedValue({ id: 'u1', displayName: 'Alice', email: 'a@example.com' })
  api.getDocComments.mockResolvedValue({ items: [], nextCursor: null })
  api.searchDirectory.mockResolvedValue({ results: [] })
  api.resolveReferences.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

function ticket() {
  return renderWithProviders(<><CommentList ticketId="t1" workspaceId="w1" /><CommentComposer ticketId="t1" workspaceId="w1" /></>)
}
const editBox = () => screen.getByRole('textbox', { name: 'Edit' })

describe('editing a ticket comment keeps what it holds (FEA-193)', () => {
  it('starts the editor from the stored body and saves everything else as it was when one line is changed', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', complex)])
    api.editTicketComment.mockResolvedValue({})
    ticket()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    // The box shows the heading, the lists, the quote and the code, not their text alone.
    expect(editBox().querySelector('h3')?.textContent).toBe('Plan')
    expect(editBox().querySelector('ol')?.getAttribute('start')).toBe('3')
    expect(editBox().querySelector('pre code')?.textContent).toBe('fn main() {}')
    // Change "four" to "five".
    const editor = editorOf(editBox())
    let from = 0
    editor.state.doc.descendants((node, pos) => {
      if (node.isText && node.text === 'four') from = pos
    })
    expect(from).toBeGreaterThan(0)
    act(() => { editor.chain().focus().setTextSelection({ from, to: from + 4 }).insertContent('five').run() })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.editTicketComment).toHaveBeenCalledTimes(1))
    const sent = api.editTicketComment.mock.calls[0]![2] as RichTextDoc
    // "four" is the one text of the second item of the ordered list; everything else is as it was stored.
    const expected = structuredClone(complex) as unknown as { content: Array<{ content?: Array<{ content: Array<{ content: Array<{ text?: string }> }> }> }> }
    expected.content[2]!.content![1]!.content[0]!.content[0]!.text = 'five'
    expect(sent).toEqual(expected)
  })

  it('saves nothing, and changes no history, when nothing was changed (Save, Escape and Cancel alike)', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', complex)])
    ticket()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Edit' })).toBeNull())
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }))
    fireEvent.keyDown(editBox(), { key: 'Escape', keyCode: 27 })
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Edit' })).toBeNull())
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }))
    replaceTextIn(editBox(), 'changed and then abandoned')
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(api.editTicketComment).not.toHaveBeenCalled()
    // The comment shows as it was.
    expect(screen.getByText('Plan')).toBeTruthy()
  })

  it('returns the keyboard to the Edit link when the box closes', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', complex)])
    ticket()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    // A browser sends the key code with the key; ProseMirror prevents the default of keyCode 27 unless it is told not to.
    fireEvent.keyDown(editBox(), { key: 'Escape', keyCode: 27 })
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Edit' })))
  })

  it('saves with Ctrl+Enter, and Enter alone breaks the line instead', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', { type: 'doc', content: [para(text('one'))] })])
    api.editTicketComment.mockResolvedValue({})
    ticket()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    typeInto(editBox(), ' more')
    fireEvent.keyDown(editBox(), { key: 'Enter', keyCode: 13 })
    expect(api.editTicketComment).not.toHaveBeenCalled()
    fireEvent.keyDown(editBox(), { key: 'Enter', keyCode: 13, ctrlKey: true })
    await waitFor(() => expect(api.editTicketComment).toHaveBeenCalledTimes(1))
    expect((api.editTicketComment.mock.calls[0]![2] as RichTextDoc).content[0]).toEqual(para(text('one more')))
  })

  it('does not save while IME composition is under way', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', { type: 'doc', content: [para(text('안녕'))] })])
    ticket()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    typeInto(editBox(), '하')
    fireEvent.keyDown(editBox(), { key: 'Enter', keyCode: 229, ctrlKey: true, isComposing: true })
    fireEvent.keyDown(editBox(), { key: 'Escape', keyCode: 229, isComposing: true })
    expect(api.editTicketComment).not.toHaveBeenCalled()
    expect(screen.queryByRole('textbox', { name: 'Edit' })).not.toBeNull()
  })

  it('keeps the box and the text when saving fails, and a second Save sends it again', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', { type: 'doc', content: [para(text('one'))] })])
    api.editTicketComment.mockRejectedValueOnce(new Error('503')).mockResolvedValue({})
    ticket()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    typeInto(editBox(), ' two')
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(await screen.findByRole('alert')).toBeTruthy()
    expect(editBox().textContent).toBe('one two')
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.editTicketComment).toHaveBeenCalledTimes(2))
    expect(api.editTicketComment.mock.calls[1]![2]).toEqual(api.editTicketComment.mock.calls[0]![2])
  })

  it('edits a body of another format as the text a person sees, and refuses an empty one', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', { schema: 'penfii.doc', version: 1, blocks: [{ id: 'b', type: 'paragraph', props: {}, content: [{ type: 'text', text: 'from a doc', styles: {} }], children: [] }] })])
    api.editTicketComment.mockResolvedValue({})
    ticket()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    expect(editBox().textContent).toBe('from a doc')
    replaceTextIn(editBox(), '')
    expect((screen.getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true)
    typeInto(editBox(), 'now text')
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.editTicketComment).toHaveBeenCalledWith('t1', 'c1', { type: 'doc', content: [para(text('now text'))] }))
  })

  it('shows a plain comment as before, line by line', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', { type: 'doc', content: [para(text('first')), para(text('second'))] })])
    ticket()
    const shown = await screen.findByText(/first/)
    expect(shown.tagName).toBe('P')
    expect(shown.textContent).toBe('first\nsecond')
  })
})

/** The body the MCP server's Markdown converter makes of a rich Markdown comment (`## Plan`, marks, a list from 3 with a nested list, a quote, a `rust` fence), verbatim. */
const fromMcp = JSON.parse('{"content":[{"attrs":{"level":2},"content":[{"text":"Plan","type":"text"}],"type":"heading"},{"content":[{"text":"First ","type":"text"},{"marks":[{"type":"bold"}],"text":"bold","type":"text"},{"text":" and ","type":"text"},{"marks":[{"type":"italic"}],"text":"italic","type":"text"},{"text":" and ","type":"text"},{"marks":[{"type":"strike"}],"text":"gone","type":"text"},{"text":" with ","type":"text"},{"marks":[{"type":"code"}],"text":"code","type":"text"},{"text":" and a ","type":"text"},{"marks":[{"attrs":{"href":"https://example.com/spec"},"type":"link"}],"text":"link","type":"text"},{"text":".","type":"text"}],"type":"paragraph"},{"attrs":{"start":3},"content":[{"content":[{"content":[{"text":"three","type":"text"}],"type":"paragraph"},{"content":[{"content":[{"content":[{"text":"nested bullet","type":"text"}],"type":"paragraph"}],"type":"listItem"}],"type":"bulletList"}],"type":"listItem"},{"content":[{"content":[{"text":"four","type":"text"}],"type":"paragraph"}],"type":"listItem"}],"type":"orderedList"},{"content":[{"content":[{"text":"quoted","type":"text"}],"type":"paragraph"}],"type":"blockquote"},{"attrs":{"language":"rust"},"content":[{"text":"fn main() {}","type":"text"}],"type":"codeBlock"}],"type":"doc"}') as RichTextDoc

describe('a comment written through MCP is edited in the web without loss (FEA-193)', () => {
  it('shows it with its formatting, saves nothing when it is not changed, and keeps all of it when one word is', async () => {
    api.getTicketComments.mockResolvedValue([ticketComment('c1', fromMcp)])
    api.editTicketComment.mockResolvedValue({})
    ticket()
    // Read mode: the formatting is there.
    expect((await screen.findByRole('heading', { name: 'Plan' })).tagName).toBe('H2')
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Edit' })).toBeNull())
    expect(api.editTicketComment).not.toHaveBeenCalled()
    // One word changed: the rest, down to the mark order, is as the MCP server stored it.
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }))
    const editor = editorOf(editBox())
    let from = 0
    editor.state.doc.descendants((node, pos) => {
      if (node.isText && node.text === 'four') from = pos
    })
    act(() => { editor.chain().focus().setTextSelection({ from, to: from + 4 }).insertContent('five').run() })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.editTicketComment).toHaveBeenCalledTimes(1))
    const expected = structuredClone(fromMcp) as unknown as { content: Array<{ content?: Array<{ content: Array<{ content: Array<{ text?: string }> }> }> }> }
    expected.content[2]!.content![1]!.content[0]!.content[0]!.text = 'five'
    expect(api.editTicketComment.mock.calls[0]![2]).toEqual(expected)
  })
})

describe('Escape in the comment editor (FEA-193, FEA-198)', () => {
  it('reaches the panel around it, so a Ticket panel closes on Escape with the keyboard in the comment box', async () => {
    api.getTicketComments.mockResolvedValue([])
    const closed = vi.fn()
    renderWithProviders(
      <div onKeyDown={(event) => { if (event.key === 'Escape' && !event.defaultPrevented) closed() }}>
        <CommentComposer ticketId="t1" workspaceId="w1" />
      </div>,
    )
    const box = screen.getByRole('textbox', { name: 'Write a comment' })
    fireEvent.keyDown(box, { key: 'Escape', keyCode: 27 })
    expect(closed).toHaveBeenCalledTimes(1)
  })

  it('belongs to the list of people while it is open, and to the page again when it is shut', async () => {
    api.getDocComments.mockResolvedValue({ items: [], nextCursor: null })
    api.searchDirectory.mockResolvedValue({ results: [{ type: 'person', id: 'u3', title: 'Cara', subtitle: '', person: { id: 'u3', displayName: 'Cara Doe', username: null, email: '', organizationId: 'o', organizationName: 'Acme', affiliation: [] } }] })
    const closed = vi.fn()
    renderWithProviders(
      <div onKeyDown={(event) => { if (event.key === 'Escape' && !event.defaultPrevented) closed() }}>
        <DocComments workspaceId="w" docId="d" userId="u1" canManage={false} refreshTick={0} />
      </div>,
    )
    const box = await screen.findByRole('textbox', { name: /Write a comment/ })
    typeInto(box, '@Ca')
    await screen.findByRole('option', { name: /Cara Doe/ })
    fireEvent.keyDown(box, { key: 'Escape', keyCode: 27 })
    await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull())
    expect(closed).not.toHaveBeenCalled()
    fireEvent.keyDown(box, { key: 'Escape', keyCode: 27 })
    expect(closed).toHaveBeenCalledTimes(1)
  })
})

describe('writing a ticket comment (FEA-193)', () => {
  const box = () => screen.getByRole('textbox', { name: 'Write a comment' })

  it('sends what was formatted, breaks the line on Enter and sends on Ctrl+Enter', async () => {
    api.getTicketComments.mockResolvedValue([])
    api.createTicketComment.mockResolvedValue({})
    ticket()
    await screen.findByText('No comments yet.')
    typeInto(box(), 'plain ')
    fireEvent.click(screen.getByRole('button', { name: 'Bold' }))
    typeInto(box(), 'strong')
    fireEvent.keyDown(box(), { key: 'Enter', keyCode: 13 })
    expect(api.createTicketComment).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Bullet list' }))
    typeInto(box(), 'item')
    fireEvent.keyDown(box(), { key: 'Enter', keyCode: 13, ctrlKey: true })
    await waitFor(() => expect(api.createTicketComment).toHaveBeenCalledTimes(1))
    const body = api.createTicketComment.mock.calls[0]![1] as RichTextDoc
    expect(JSON.stringify(body)).toContain('"marks":[{"type":"bold"}]')
    expect(body.content.map((block) => block.type)).toEqual(['paragraph', 'bulletList'])
    // The box is empty again and the person can go on.
    await waitFor(() => expect(box().textContent).toBe(''))
  })

  it('does not post an empty comment, nor one of blank lines', async () => {
    api.getTicketComments.mockResolvedValue([])
    ticket()
    await screen.findByText('No comments yet.')
    expect((screen.getByRole('button', { name: 'Comment' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.keyDown(box(), { key: 'Enter', keyCode: 13, ctrlKey: true })
    typeInto(box(), '   ')
    fireEvent.keyDown(box(), { key: 'Enter', keyCode: 13, ctrlKey: true })
    expect(api.createTicketComment).not.toHaveBeenCalled()
  })

  it('keeps the text when posting fails', async () => {
    api.getTicketComments.mockResolvedValue([])
    api.createTicketComment.mockRejectedValue(new Error('boom'))
    ticket()
    await screen.findByText('No comments yet.')
    typeInto(box(), 'keep me')
    fireEvent.click(screen.getByRole('button', { name: 'Comment' }))
    expect(await screen.findByRole('alert')).toBeTruthy()
    expect(box().textContent).toBe('keep me')
  })
})

describe('editing a Doc comment keeps its mentions (FEA-193)', () => {
  const body = { type: 'doc', content: [para(mention('occ-1', 'u2'), text(' please look at '), text('this', [{ type: 'bold' }]), text(' and write @nobody'))] }
  const docComment = { id: 'c1', authorId: 'u1', authorName: 'Alice', createdAt: '2026-10-20T00:00:00Z', editedAt: null, body, attachments: [] }

  function docs() {
    return renderWithProviders(<DocComments workspaceId="w" docId="d" userId="u1" canManage={false} refreshTick={0} />)
  }

  it('saves a changed line with the mention as it was (same person, same occurrence) and the typed @name as plain text', async () => {
    api.getDocComments.mockResolvedValue({ items: [docComment], nextCursor: null })
    api.resolveReferences.mockResolvedValue([{ kind: 'USER', id: 'u2', available: true, title: 'Bob Lee' }])
    api.editDocComment.mockResolvedValue({})
    docs()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    const box = screen.getByRole('textbox', { name: 'Edit' })
    typeInto(box, '!')
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.editDocComment).toHaveBeenCalledTimes(1))
    const sent = api.editDocComment.mock.calls[0]![2] as { content: Array<{ content: Array<{ type: string; attrs?: { occurrenceId: string; userId: string }; text?: string }> }> }
    const inline = sent.content[0]!.content
    expect(inline.filter((node) => node.type === 'mention')).toEqual([{ type: 'mention', attrs: { occurrenceId: 'occ-1', kind: 'USER', userId: 'u2' } }])
    expect(inline.filter((node) => node.type === 'text').map((node) => node.text).join('')).toContain('@nobody')
  })

  it('adds a mention picked while editing, with a new occurrence, next to the one that was there', async () => {
    api.getDocComments.mockResolvedValue({ items: [docComment], nextCursor: null })
    api.searchDirectory.mockResolvedValue({ results: [{ type: 'person', id: 'u3', title: 'Cara', subtitle: '', person: { id: 'u3', displayName: 'Cara Doe', username: null, email: '', organizationId: 'o', organizationName: 'Acme', affiliation: [] } }] })
    api.editDocComment.mockResolvedValue({})
    docs()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    const box = screen.getByRole('textbox', { name: 'Edit' })
    typeInto(box, ' @Ca')
    fireEvent.click(await screen.findByRole('option', { name: /Cara Doe/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.editDocComment).toHaveBeenCalledTimes(1))
    const mentions = JSON.stringify(api.editDocComment.mock.calls[0]![2]).match(/"type":"mention","attrs":\{[^}]*\}/g) ?? []
    expect(mentions).toHaveLength(2)
    expect(mentions[0]).toContain('"occurrenceId":"occ-1"')
    expect(mentions[1]).toContain('"userId":"u3"')
    expect(mentions[1]).not.toContain('occ-1')
  })

  it('sends nothing when an unchanged comment is saved', async () => {
    api.getDocComments.mockResolvedValue({ items: [docComment], nextCursor: null })
    docs()
    fireEvent.click(await screen.findByRole('button', { name: 'Edit' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Edit' })).toBeNull())
    expect(api.editDocComment).not.toHaveBeenCalled()
  })

  it('posts a new comment with formatting and keeps it when posting fails', async () => {
    api.createDocComment.mockRejectedValueOnce(new Error('503')).mockResolvedValue({})
    docs()
    const box = await screen.findByRole('textbox', { name: /Write a comment/ })
    typeInto(box, 'first try')
    fireEvent.keyDown(box, { key: 'Enter', keyCode: 13, ctrlKey: true })
    expect(await screen.findByRole('alert')).toBeTruthy()
    expect(box.textContent).toBe('first try')
    fireEvent.click(screen.getByRole('button', { name: 'Comment' }))
    await waitFor(() => expect(api.createDocComment).toHaveBeenCalledTimes(2))
    expect(api.createDocComment.mock.calls[1]![1]).toEqual({ type: 'doc', content: [para(text('first try'))] })
  })
})
