import { describe, expect, it } from 'vitest'
import type { DocContent } from '@penfii/contracts'
import { toBlocks, toContent } from './draftModel'

const doc = (content: unknown[]): DocContent => ({
  schema: 'penfii.doc',
  version: 1,
  blocks: [{ id: 'b1', type: 'paragraph', props: {}, content, children: [{ id: 'b2', type: 'paragraph', props: {}, content, children: [] }] }],
})

describe('draft model', () => {
  it('keeps only code on a run that has code and other styles, at any depth, and leaves the rest alone', () => {
    const blocks = toBlocks(
      doc([
        { type: 'text', text: 'a', styles: { bold: true, code: true } },
        { type: 'text', text: 'b', styles: { bold: true } },
        { type: 'link', href: 'https://x.test', content: [{ type: 'text', text: 'c', styles: { italic: true, code: true } }] },
      ]),
    ) as Array<{ content: Array<Record<string, unknown>>; children: Array<{ content: unknown[] }> }>
    expect(blocks[0]!.content[0]).toEqual({ type: 'text', text: 'a', styles: { code: true } })
    expect(blocks[0]!.content[1]).toEqual({ type: 'text', text: 'b', styles: { bold: true } })
    expect((blocks[0]!.content[2] as { content: unknown[] }).content[0]).toEqual({ type: 'text', text: 'c', styles: { code: true } })
    expect(blocks[0]!.children[0]!.content[0]).toEqual({ type: 'text', text: 'a', styles: { code: true } })
  })

  it('does not change the content it was given', () => {
    const content = doc([{ type: 'text', text: 'a', styles: { bold: true, code: true } }])
    const before = JSON.stringify(content)
    toBlocks(content)
    expect(JSON.stringify(content)).toBe(before)
  })

  it('starts an empty page from one empty paragraph and wraps blocks as DocContent v1', () => {
    const blocks = toBlocks({ schema: 'penfii.doc', version: 1, blocks: [] })
    expect(blocks).toHaveLength(1)
    expect(toContent(blocks)).toEqual({ schema: 'penfii.doc', version: 1, blocks })
  })
})
