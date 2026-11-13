import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'
import { LOCALE_STORAGE_KEY } from '../../i18n/messages'
import { AuthorCard } from '../people/AuthorCard'
import { ChannelWebhooksButton } from './ChannelWebhooks'

vi.mock('../../api', () => ({
  api: {
    listChannelWebhooks: vi.fn(),
    createChannelWebhook: vi.fn(),
    rotateWebhook: vi.fn(),
    revokeWebhook: vi.fn(),
    reauthPassword: vi.fn(),
  },
}))

import { api } from '../../api'

const list = vi.mocked(api.listChannelWebhooks)
const create = vi.mocked(api.createChannelWebhook)

function renderWebhooks(canIssue = true) {
  localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <I18nProvider>
        <ChannelWebhooksButton channelId="channel-1" canIssue={canIssue} />
      </I18nProvider>
    </QueryClientProvider>,
  )
}

describe('channel webhooks', () => {
  beforeEach(() => {
    list.mockReset()
    create.mockReset()
    list.mockResolvedValue({ items: [] })
  })

  it('shows a secret only until the person dismisses it', async () => {
    create.mockResolvedValue({
      secret: 'shown-once',
      webhook: {
        id: 'w1',
        channelId: 'channel-1',
        name: 'deploys',
        authMode: 'URL_SECRET',
        actorId: 'a1',
        actorDisplayName: 'Deploy Bot',
        revoked: false,
        createdAt: '2026-11-09T00:00:00Z',
        rotatedAt: null,
      },
    })
    renderWebhooks()
    fireEvent.click(screen.getByRole('button', { name: 'Webhooks' }))
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'deploys' } })
    fireEvent.change(screen.getByLabelText('Author name'), { target: { value: 'Deploy Bot' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create webhook' }))
    expect(await screen.findByText('shown-once')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Done' }))
    expect(screen.queryByText('shown-once')).toBeNull()
    expect(list).toHaveBeenCalled()
  })
})

describe('an admin who cannot read the channel', () => {
  it('sees the webhooks and revokes one, but cannot create or replace a credential', async () => {
    list.mockResolvedValue({
      items: [{ id: 'w1', channelId: 'channel-1', name: 'deploys', authMode: 'URL_SECRET', actorId: 'a1', actorDisplayName: 'Deploy Bot', revoked: false, createdAt: '2026-11-09T00:00:00Z', rotatedAt: null }],
    })
    renderWebhooks(false)
    fireEvent.click(screen.getByRole('button', { name: 'Webhooks' }))
    expect(await screen.findByText('deploys')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Revoke' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Rotate secret' }), 'rotating issues a new posting credential').toBeNull()
    expect(screen.queryByRole('button', { name: 'Create webhook' })).toBeNull()
    expect(screen.getByText(/can see and revoke its webhooks only/)).toBeTruthy()
  })
})

describe('integration authors', () => {
  it('are a name, with no way to start a direct message', () => {
    localStorage.setItem(LOCALE_STORAGE_KEY, 'en-US')
    const view = render(
      <QueryClientProvider client={new QueryClient()}>
        <MemoryRouter initialEntries={['/w/ws/c/c1']}>
          <Routes>
            <Route
              path="/w/:workspaceId/c/:conversationId"
              element={
                <I18nProvider>
                  <AuthorCard workspaceId="ws" userId="bot" name="Deploy Bot" authorKind="INTEGRATION" />
                </I18nProvider>
              }
            />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>,
    )
    expect(view.getByText('Deploy Bot').tagName).toBe('STRONG')
    expect(view.queryByRole('button', { name: 'Send a message' })).toBeNull()
    view.unmount()

    const person = render(
      <QueryClientProvider client={new QueryClient()}>
        <MemoryRouter>
          <I18nProvider>
            <AuthorCard workspaceId="ws" userId="human" name="Ada" authorKind="HUMAN" />
          </I18nProvider>
        </MemoryRouter>
      </QueryClientProvider>,
    )
    expect(person.getByRole('button', { name: 'Ada' })).toBeTruthy()
  })
})

describe('the webhook dialog as a modal', () => {
  it('is a native modal dialog that Escape closes', async () => {
    list.mockResolvedValue({ items: [] } as never)
    const { container } = renderWebhooks()
    fireEvent.click(screen.getByRole('button', { name: /웹훅|webhooks/i }))
    const dialog = await screen.findByRole('dialog')
    expect(dialog.tagName).toBe('DIALOG')
    fireEvent(dialog, new Event('cancel', { cancelable: true }))
    await waitFor(() => expect(container.ownerDocument.querySelector('dialog[open]')).toBeNull())
  })
})
