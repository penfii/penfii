// @vitest-environment jsdom
// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import { clearAllNewDocDrafts, isBlankDraft, readNewDocDraft, writeNewDocDraft } from './newDocDraft'

const content = { schema: 'penfii.doc' as const, version: 1 as const, blocks: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }] }

afterEach(() => localStorage.clear())

describe('new document draft', () => {
  it('keeps one draft for a person and workspace and drops files', () => {
    const withFile = { ...content, blocks: [...content.blocks, { type: 'image', props: { url: '/v1/files/x/content' } }] }
    expect(writeNewDocDraft('u1', 'w', { title: '노트', content: withFile })).toBe(true)
    const saved = readNewDocDraft('u1', 'w')
    expect(saved?.title).toBe('노트')
    expect(saved?.content.blocks.some((block) => (block as { type?: string }).type === 'image')).toBe(false)
    expect(readNewDocDraft('u2', 'w')).toBeNull()
    writeNewDocDraft('u1', 'w2', { title: '다른 곳', content })
    clearAllNewDocDrafts()
    expect(readNewDocDraft('u1', 'w')).toBeNull()
    expect(readNewDocDraft('u1', 'w2')).toBeNull()
  })

  it('removes a blank draft', () => {
    writeNewDocDraft('u1', 'w', { title: '남김', content })
    expect(writeNewDocDraft('u1', 'w', { title: '  ', content: { schema: 'penfii.doc', version: 1, blocks: [{ type: 'paragraph', content: [] }] } })).toBe(true)
    expect(readNewDocDraft('u1', 'w')).toBeNull()
    expect(isBlankDraft({ title: '', content: { schema: 'penfii.doc', version: 1, blocks: [] } })).toBe(true)
  })
})
