import { describe, expect, it } from 'vitest'
import { isFormattedRichText } from './text'

const p = (text: string, marks?: unknown[]) => ({ type: 'paragraph', content: [{ type: 'text', text, ...(marks ? { marks } : {}) }] })

describe('isFormattedRichText', () => {
  it('is false for plain paragraphs, which keep their old rendering', () => {
    expect(isFormattedRichText({ type: 'doc', content: [p('a'), { type: 'paragraph' }, p('b')] })).toBe(false)
    expect(isFormattedRichText({ type: 'doc', content: [] })).toBe(false)
  })

  it('is true for a heading, a list, a quote, code or marked text', () => {
    expect(isFormattedRichText({ type: 'doc', content: [{ type: 'heading', attrs: { level: 2 }, content: [{ type: 'text', text: 'h' }] }] })).toBe(true)
    expect(isFormattedRichText({ type: 'doc', content: [{ type: 'bulletList', content: [{ type: 'listItem', content: [p('x')] }] }] })).toBe(true)
    expect(isFormattedRichText({ type: 'doc', content: [{ type: 'codeBlock', content: [{ type: 'text', text: 'x' }] }] })).toBe(true)
    expect(isFormattedRichText({ type: 'doc', content: [p('x', [{ type: 'bold' }])] })).toBe(true)
  })

  it('is false when the view would drop something, so the plain text is shown instead', () => {
    expect(isFormattedRichText({ type: 'doc', content: [{ type: 'table', content: [] }, { type: 'bulletList', content: [] }] })).toBe(false)
    expect(isFormattedRichText({ type: 'doc', content: [p('x', [{ type: 'underline' }])] })).toBe(false)
    expect(isFormattedRichText({ schema: 'penfii.doc', version: 1, blocks: [] })).toBe(false)
    expect(isFormattedRichText(null)).toBe(false)
    expect(isFormattedRichText('text')).toBe(false)
  })
})

describe('isEditableRichText', () => {
  it('is true for what the editor carries whole, plain paragraphs and mentions included', async () => {
    const { isEditableRichText } = await import('./text')
    expect(isEditableRichText({ type: 'doc', content: [p('a'), { type: 'paragraph' }] })).toBe(true)
    expect(isEditableRichText({ type: 'doc', content: [] })).toBe(true)
    expect(isEditableRichText({ type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'mention', attrs: { occurrenceId: 'o', kind: 'USER', userId: 'u' } }] }] })).toBe(true)
    expect(isEditableRichText({ type: 'doc', content: [{ type: 'orderedList', attrs: { start: 3 }, content: [{ type: 'listItem', content: [p('x', [{ type: 'link', attrs: { href: 'https://example.com' } }])] }] }] })).toBe(true)
  })

  it('is false for another document format and for a node the editor would drop', async () => {
    const { isEditableRichText } = await import('./text')
    expect(isEditableRichText({ schema: 'penfii.doc', version: 1, blocks: [] })).toBe(false)
    expect(isEditableRichText({ type: 'doc', content: [{ type: 'table', content: [] }] })).toBe(false)
    expect(isEditableRichText({ type: 'doc', content: [p('x', [{ type: 'underline' }])] })).toBe(false)
    expect(isEditableRichText(null)).toBe(false)
  })
})
