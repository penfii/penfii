import { describe, expect, it } from 'vitest'
import { BlockNoteEditor } from '@blocknote/core'
import { descriptionText } from './ticketModel'
import { ticketDescriptionToDocContent } from './ticketDescriptionContent'
import { docSchema } from '../docs/schema'

describe('Ticket description conversion to the Docs BlockNote model', () => {
  it('keeps current Docs DocContent unchanged', () => {
    const content = { schema: 'penfii.doc', version: 1, blocks: [{ type: 'paragraph', props: {}, content: [{ type: 'text', text: 'current', styles: {} }], children: [] }] }
    expect(ticketDescriptionToDocContent(content)).toBe(content)
  })

  it('converts legacy paragraphs, headings, code and inline marks into the Docs schema', () => {
    const content = ticketDescriptionToDocContent({ type: 'doc', content: [
      { type: 'heading', attrs: { level: 2 }, content: [{ type: 'text', text: 'Heading', marks: [{ type: 'bold' }] }] },
      { type: 'paragraph', content: [{ type: 'text', text: 'line one' }, { type: 'hardBreak' }, { type: 'text', text: 'line two' }] },
      { type: 'codeBlock', content: [{ type: 'text', text: 'echo ok' }] },
    ] })
    const editor = BlockNoteEditor.create({ schema: docSchema })
    expect(() => editor.replaceBlocks(editor.document, content.blocks as any)).not.toThrow()
    expect(content.blocks.map((block: any) => block.type)).toEqual(['heading', 'paragraph', 'codeBlock'])
    expect((content.blocks[0] as any).props.level).toBe(2)
    expect(descriptionText(content)).toContain('Heading\nline one\nline two\necho ok')
  })

  it('represents an empty legacy document as a valid blank Docs paragraph', () => {
    const content = ticketDescriptionToDocContent({ type: 'doc', content: [] })
    const editor = BlockNoteEditor.create({ schema: docSchema })
    expect(() => editor.replaceBlocks(editor.document, content.blocks as any)).not.toThrow()
    expect(content.blocks).toHaveLength(1)
  })

  it('keeps unknown legacy nodes and inline content visible instead of silently dropping them', () => {
    const content = ticketDescriptionToDocContent({ type: 'doc', content: [
      { type: 'futureWidget', attrs: { widgetId: 'widget-1' } },
      { type: 'paragraph', content: [{ type: 'futureInline', attrs: { payload: 'keep-me' } }] },
    ] })
    const text = descriptionText(content)
    expect(text).toContain('futureWidget')
    expect(text).toContain('widget-1')
    expect(text).toContain('futureInline')
    expect(text).toContain('keep-me')
    const editor = BlockNoteEditor.create({ schema: docSchema })
    expect(() => editor.replaceBlocks(editor.document, content.blocks as any)).not.toThrow()
  })

  it('keeps nested lists, later paragraphs of an item, task state, quotes and code language', () => {
    const content = ticketDescriptionToDocContent({ type: 'doc', content: [
      { type: 'bulletList', content: [
        { type: 'listItem', content: [
          { type: 'paragraph', content: [{ type: 'text', text: 'parent' }] },
          { type: 'paragraph', content: [{ type: 'text', text: 'second paragraph' }] },
          { type: 'orderedList', content: [{ type: 'listItem', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'nested' }] }] }] },
        ] },
      ] },
      { type: 'taskList', content: [{ type: 'taskItem', attrs: { checked: true }, content: [{ type: 'paragraph', content: [{ type: 'text', text: 'done' }] }] }] },
      { type: 'blockquote', content: [
        { type: 'paragraph', content: [{ type: 'text', text: 'quoted', marks: [{ type: 'italic' }] }] },
        { type: 'paragraph', content: [{ type: 'text', text: 'again' }] },
      ] },
      { type: 'codeBlock', attrs: { language: 'rust' }, content: [{ type: 'text', text: 'fn main() {}' }] },
    ] })
    const [parent, task, quote, code] = content.blocks as any[]
    expect(parent).toMatchObject({ type: 'bulletListItem', content: [{ text: 'parent' }] })
    expect(parent.children.map((child: any) => child.type)).toEqual(['paragraph', 'numberedListItem'])
    expect(parent.children[1].content[0].text).toBe('nested')
    expect(task).toMatchObject({ type: 'checkListItem', props: { checked: true } })
    expect(quote.content).toEqual([
      { type: 'text', text: 'quoted', styles: { italic: true } },
      { type: 'text', text: '\n', styles: {} },
      { type: 'text', text: 'again', styles: {} },
    ])
    expect(code).toMatchObject({ type: 'codeBlock', props: { language: 'rust' }, content: [{ text: 'fn main() {}' }] })
    const editor = BlockNoteEditor.create({ schema: docSchema })
    expect(() => editor.replaceBlocks(editor.document, content.blocks as any)).not.toThrow()
  })

  it('turns only safe link marks into links and keeps invalid mentions and references as text', () => {
    const userId = '7ae37e8a-8ce0-4ccb-9aed-4a870e9d9155'
    const content = ticketDescriptionToDocContent({ type: 'doc', content: [{ type: 'paragraph', content: [
      { type: 'text', text: 'site', marks: [{ type: 'link', attrs: { href: 'https://example.com' } }] },
      { type: 'text', text: 'script', marks: [{ type: 'link', attrs: { href: 'javascript:alert(1)' } }] },
      { type: 'mention', attrs: { userId } },
      { type: 'mention', attrs: { userId: 'not-an-id' } },
      { type: 'reference', attrs: { kind: 'USER', id: userId } },
    ] }] })
    const inline = (content.blocks[0] as any).content
    expect(inline[0]).toEqual({ type: 'link', href: 'https://example.com', content: [{ type: 'text', text: 'site', styles: {} }] })
    expect(inline[1]).toEqual({ type: 'text', text: 'script', styles: {} })
    expect(inline[2]).toEqual({ type: 'mention', props: { userId } })
    expect(inline[3].text).toContain('not-an-id')
    expect(inline[4].text).toContain('USER')
  })
})
