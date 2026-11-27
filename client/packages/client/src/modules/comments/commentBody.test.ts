import type { RichTextDoc } from '@penfii/contracts'
import { describe, expect, it } from 'vitest'
import { editableBody, paragraphsOf, showsRich, withoutBlankEdges } from './commentBody'

const doc = (...content: unknown[]) => ({ type: 'doc', content }) as unknown as RichTextDoc
const p = (text?: string) => ({ type: 'paragraph', content: text === undefined ? [] : [{ type: 'text', text }] })

describe('comment bodies', () => {
  it('makes one paragraph per line, with an empty one for a blank line', () => {
    expect(paragraphsOf('a\n\nb')).toEqual(doc(p('a'), p(), p('b')))
    expect(paragraphsOf('a  \n ')).toEqual(doc(p('a')))
  })

  it('edits a body the editor carries as it is, and any other body as its plain text', () => {
    const rich = doc({ type: 'heading', attrs: { level: 2 }, content: [{ type: 'text', text: 'T' }] }, p('x'))
    expect(editableBody(rich, () => 'unused')).toBe(rich)
    const other = { schema: 'penfii.doc', version: 1, blocks: [] }
    expect(editableBody(other, () => 'one\ntwo')).toEqual(doc(p('one'), p('two')))
  })

  it('shows the rich view for formatting and for a mention, and lines of text for plain paragraphs', () => {
    expect(showsRich(doc(p('plain')))).toBe(false)
    expect(showsRich(doc({ type: 'blockquote', content: [p('q')] }))).toBe(true)
    expect(showsRich(doc({ type: 'paragraph', content: [{ type: 'mention', attrs: { occurrenceId: 'o', kind: 'USER', userId: 'u' } }] }))).toBe(true)
    expect(showsRich({ schema: 'penfii.doc', version: 1, blocks: [] })).toBe(false)
  })

  it('drops the empty paragraphs at the two ends and keeps those between', () => {
    expect(withoutBlankEdges(doc(p(), p('a'), p(), p('b'), p(' '), p()))).toEqual(doc(p('a'), p(), p('b')))
    expect(withoutBlankEdges(doc(p(), p()))).toEqual(doc())
    const quoted = doc({ type: 'blockquote', content: [p()] })
    expect(withoutBlankEdges(quoted)).toEqual(quoted)
  })
})
