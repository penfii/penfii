import { describe, expect, it } from 'vitest'
import type { Message } from '@penfii/contracts'
import { prefillFromMessage } from './fromChat'

function message(text: string, extra: unknown[] = []): Message {
  return { id: 'm', body: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }, ...extra] }] } } as unknown as Message
}

describe('a Ticket started from a message', () => {
  it('takes the message text as it is', () => {
    expect(prefillFromMessage(message('배포 일정 확인'), {})).toEqual({ title: '배포 일정 확인', description: '' })
  })

  it('uses the first line as the title and keeps the whole text as the description', () => {
    const body = { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: '첫 줄' }] }, { type: 'paragraph', content: [{ type: 'text', text: '둘째 줄' }] }] }
    expect(prefillFromMessage({ id: 'm', body } as unknown as Message, {})).toEqual({ title: '첫 줄', description: '첫 줄\n둘째 줄' })
  })

  it('cuts a long first line and shows a mention by name', () => {
    const long = 'a'.repeat(200)
    const cut = prefillFromMessage(message(long), {})
    expect(cut.title.length).toBe(120)
    expect(cut.description).toBe(long)
    const id = '11111111-1111-1111-1111-111111111111'
    const mention = prefillFromMessage(message('cc ', [{ type: 'mention', attrs: { userId: id, occurrenceId: 'o' } }]), { [id]: 'Kim' })
    expect(mention.title).toBe('cc @Kim')
  })
})
