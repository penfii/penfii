import { describe, expect, it } from 'vitest'
import { BlockNoteEditor } from '@blocknote/core'
import { GOLDEN_CASES } from './golden'
import { toBlocks, toContent } from './draftModel'
import { docSchema } from './schema'

/** The plain editor must load every kind of page the server can hold, without the Yjs layer. */
describe('draft in the plain editor', () => {
  it('loads every golden case and gives DocContent back', () => {
    for (const { name, blocks } of GOLDEN_CASES) {
      const editor = BlockNoteEditor.create({ schema: docSchema })
      expect(() => editor.replaceBlocks(editor.document, toBlocks({ schema: 'penfii.doc', version: 1, blocks }) as any), name).not.toThrow()
      const back = toContent(editor.document)
      expect(back.schema).toBe('penfii.doc')
      expect(back.blocks.length, name).toBeGreaterThan(0)
    }
  })

  it('needs the code-alone rule: the editor refuses code mixed with another style', () => {
    const mixed = GOLDEN_CASES.find((c) => c.name === 'every style')!
    const editor = BlockNoteEditor.create({ schema: docSchema })
    expect(() => editor.replaceBlocks(editor.document, mixed.blocks as any)).toThrow()
  })
})
