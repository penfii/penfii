import type { RichTextDoc } from '@penfii/contracts'
import { Editor } from '@tiptap/core'
import { describe, expect, it } from 'vitest'
import { EMPTY_RICH_TEXT } from './empty'
import { editorToRichText, richTextToEditor, sameRichText } from './adapter'
import { editorExtensions } from './editorExtensions'
import { parseRichText } from './parse'

function throughEditor(doc: RichTextDoc): RichTextDoc {
  const editor = new Editor({
    extensions: editorExtensions(() => ''),
    content: richTextToEditor(doc),
  })
  const stored = editorToRichText(editor.getJSON(), doc)
  editor.destroy()
  return stored
}

function roundTrip(doc: RichTextDoc, baseline?: RichTextDoc): RichTextDoc {
  return editorToRichText(richTextToEditor(doc), baseline ?? null)
}

describe('rich text adapter', () => {
  it('keeps an existing paragraph, marks, and link', () => {
    const doc: RichTextDoc = {
      type: 'doc',
      content: [{
        type: 'paragraph',
        content: [
          { type: 'text', text: 'hello ', marks: [{ type: 'bold' }, { type: 'italic' }] },
          { type: 'text', text: 'gone', marks: [{ type: 'strike' }] },
          { type: 'text', text: ' site', marks: [{ type: 'link', attrs: { href: 'https://example.com' } }] },
        ],
      }],
    }
    expect(roundTrip(doc, doc)).toBe(doc)
  })

  it('drops a javascript link mark', () => {
    const doc: RichTextDoc = {
      type: 'doc',
      content: [{
        type: 'paragraph',
        content: [{ type: 'text', text: 'x', marks: [{ type: 'link', attrs: { href: 'javascript:alert(1)' } }] }],
      }],
    }
    const stored = roundTrip(doc)
    expect(stored.content[0]).toEqual({ type: 'paragraph', content: [{ type: 'text', text: 'x' }] })
  })

  it('preserves a legacy inline quote when nothing changed', () => {
    const legacy = {
      type: 'doc',
      content: [{ type: 'blockquote', content: [{ type: 'text', text: 'quoted' }] }],
    } as unknown as RichTextDoc
    expect(roundTrip(legacy, legacy)).toBe(legacy)
    const rewritten = roundTrip(legacy)
    expect(rewritten.content[0]).toEqual({
      type: 'blockquote',
      content: [{ type: 'paragraph', content: [{ type: 'text', text: 'quoted' }] }],
    })
  })

  it('stores a nested list and quote', () => {
    const source = parseRichText('> quoted\n\n- one\n- two\n\n1. first')
    const stored = roundTrip(source)
    expect(stored.content.map((block) => block.type)).toEqual(['blockquote', 'bulletList', 'orderedList'])
    expect(sameRichText(stored, source)).toBe(true)
  })

  it('stores code with and without a language', () => {
    const plain = parseRichText('```\nplain\n```')
    const typed = parseRichText('```ts\nconst n = 1\n```')
    expect(roundTrip(plain).content[0]).toMatchObject({ type: 'codeBlock', attrs: { language: null } })
    expect(roundTrip(typed).content[0]).toMatchObject({ type: 'codeBlock', attrs: { language: 'ts' } })
    expect(roundTrip(typed, typed)).toBe(typed)
  })

  it('keeps a pasted document stable after the editor reloads it', () => {
    const source = parseRichText('> md-quote\n\n- md-bullet\n\n```\nmd-code\n```')
    const editor = new Editor({
      extensions: editorExtensions(() => ''),
      content: richTextToEditor(EMPTY_RICH_TEXT),
    })
    editor.commands.insertContent(richTextToEditor(source).content ?? [])
    const first = editorToRichText(editor.getJSON(), null)
    editor.commands.setContent(richTextToEditor(first), { emitUpdate: false })
    const second = editorToRichText(editor.getJSON(), null)
    editor.destroy()
    expect(sameRichText(first, second)).toBe(true)
    expect(sameRichText(first, source)).toBe(true)
  })

  it('opens existing documents in Tiptap and saves the same JSON', () => {
    const samples: RichTextDoc[] = [
      {
        type: 'doc',
        content: [{
          type: 'paragraph',
          content: [
            { type: 'text', text: 'hello ', marks: [{ type: 'bold' }] },
            { type: 'text', text: 'there', marks: [{ type: 'italic' }, { type: 'strike' }] },
            { type: 'text', text: ' link', marks: [{ type: 'link', attrs: { href: 'https://example.com/a' } }] },
          ],
        }],
      },
      parseRichText('> quoted line\n\n- alpha\n- beta\n\n1. first\n2. second'),
      parseRichText('```\nplain code\n```'),
      parseRichText('```ts\nconst n = 1\n```'),
      parseRichText('###### small heading'),
    ]
    for (const sample of samples) expect(throughEditor(sample)).toBe(sample)
  })

  it('turns a hard break back into a newline without changing stored text', () => {
    const doc: RichTextDoc = {
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text: 'one\ntwo' }] }],
    }
    expect(roundTrip(doc, doc)).toBe(doc)
  })
})
