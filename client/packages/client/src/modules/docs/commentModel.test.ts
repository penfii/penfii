import { describe, expect, it } from 'vitest'
import { commentToRichText } from './commentModel'

let n = 0
const id = () => `occ-${++n}`

describe('comment to RichText v1', () => {
  it('turns picked @names into mention nodes and keeps the rest as text', () => {
    n = 0
    const doc = commentToRichText('@김민수 확인 부탁, 그리고 @이영희 도 @아무개', [
      { userId: 'u1', name: '김민수' },
      { userId: 'u2', name: '이영희' },
    ], id)
    expect(doc.content[0]).toEqual({
      type: 'paragraph',
      content: [
        { type: 'mention', attrs: { occurrenceId: 'occ-1', kind: 'USER', userId: 'u1' } },
        { type: 'text', text: ' 확인 부탁, 그리고 ' },
        { type: 'mention', attrs: { occurrenceId: 'occ-2', kind: 'USER', userId: 'u2' } },
        { type: 'text', text: ' 도 @아무개' },
      ],
    })
  })

  it('keeps one paragraph per line and handles an empty line', () => {
    const doc = commentToRichText('첫 줄\n\n셋째', [], id)
    expect(doc.content.map((p) => (p as { content: unknown[] }).content.length)).toEqual([1, 0, 1])
  })

  it('prefers the longer of two names that share a start', () => {
    const doc = commentToRichText('@김민 과 @김민수', [{ userId: 'a', name: '김민' }, { userId: 'b', name: '김민수' }], id)
    const kinds = (doc.content[0] as { content: Array<{ attrs?: { userId: string } }> }).content.filter((c) => c.attrs).map((c) => c.attrs!.userId)
    expect(kinds).toEqual(['a', 'b'])
  })
})
