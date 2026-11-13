import { render } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router'
import { describe, expect, it } from 'vitest'
import type { Message } from '@penfii/contracts'
import { I18nProvider } from '../../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../../i18n/messages'
import { MessageRow } from './MessageRow'

function message(patch: Partial<Message>): Message {
  return {
    id: 'm1',
    conversationId: 'c1',
    streamSeq: 2,
    authorId: 'u1',
    authorDisplayName: 'Alice',
    authorKind: 'HUMAN',
    threadRootId: null,
    replyToId: null,
    version: 1,
    bodySchemaVersion: 1,
    body: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }] },
    deleted: false,
    createdAt: '2026-11-05T02:00:00Z',
    editedAt: null,
    replyCount: 0,
    reactions: [],
    attachments: [],
    ...patch,
  } as Message
}

function row(patch: Partial<Message>, compact: boolean) {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <MemoryRouter>
        <I18nProvider>
          <MessageRow
            message={message(patch)}
            currentUserId="u1"
            names={{}}
            scrollRoot={null}
            onVisible={() => undefined}
            onMessage={() => undefined}
            onNotFound={() => undefined}
            onManualUnread={() => undefined}
            compact={compact}
          />
        </I18nProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('the edited marker', () => {
  it('shows on a grouped message, which has no header to carry it', () => {
    const view = row({ editedAt: '2026-11-05T02:01:00Z', version: 2 }, true)
    expect(view.container.querySelector('.message-meta')).toBeNull()
    expect(view.container.querySelectorAll('.edited-link')).toHaveLength(1)
  })

  it('shows once in the header of a message that starts a group', () => {
    const view = row({ editedAt: '2026-11-05T02:01:00Z', version: 2 }, false)
    expect(view.container.querySelectorAll('.message-meta .edited-link')).toHaveLength(1)
    expect(view.container.querySelectorAll('.edited-link')).toHaveLength(1)
  })

  it('is a history link for the author only, because the server refuses everyone else', () => {
    const own = row({ editedAt: '2026-11-05T02:01:00Z', version: 2 }, true)
    expect(own.container.querySelector('button.edited-link')).not.toBeNull()
    own.unmount()
    const other = row({ editedAt: '2026-11-05T02:01:00Z', version: 2, authorId: 'u2' }, true)
    expect(other.container.querySelector('button.edited-link')).toBeNull()
    expect(other.container.querySelector('span.edited-link')?.textContent).toBe('edited')
  })

  it('is absent from a message that was never edited or was deleted', () => {
    expect(row({}, true).container.querySelector('.edited-link')).toBeNull()
    expect(
      row({ editedAt: '2026-11-05T02:01:00Z', deleted: true, body: null }, true).container.querySelector('.edited-link'),
    ).toBeNull()
  })
})
