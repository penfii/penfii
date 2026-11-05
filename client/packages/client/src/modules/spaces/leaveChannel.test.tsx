import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '@penfii/api-client'
import { I18nProvider } from '../../i18n/I18nProvider'

const api = vi.hoisted(() => ({ leaveChannel: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { LeaveChannelButton } from './LeaveChannelButton'

function mount(onLeft = vi.fn(), onChooseManager?: () => void) {
  render(
    <I18nProvider>
      <LeaveChannelButton channelId="ch" channelName="leads" userId="u1" onLeft={onLeft} onChooseManager={onChooseManager} />
    </I18nProvider>,
  )
  return onLeft
}

const confirmLeaving = async () => {
  fireEvent.click(screen.getByRole('button', { name: 'Leave channel' }))
  fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Leave channel' }))
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  HTMLDialogElement.prototype.showModal = HTMLDialogElement.prototype.showModal ?? function (this: HTMLDialogElement) { this.setAttribute('open', '') }
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('Leaving a private channel', () => {
  it('asks first, then removes only the person themselves and lets the page move on', async () => {
    api.leaveChannel.mockResolvedValue(undefined)
    const onLeft = mount()
    await confirmLeaving()
    await waitFor(() => expect(api.leaveChannel).toHaveBeenCalledWith('ch', 'u1'))
    await waitFor(() => expect(onLeft).toHaveBeenCalledTimes(1))
  })

  it('does nothing when the question is declined', async () => {
    const onLeft = mount()
    fireEvent.click(screen.getByRole('button', { name: 'Leave channel' }))
    fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(api.leaveChannel).not.toHaveBeenCalled()
    expect(onLeft).not.toHaveBeenCalled()
  })

  it('says why the last manager cannot leave, and stays', async () => {
    api.leaveChannel.mockRejectedValue(new ApiError(409, 'last_manager', 'the last manager cannot leave'))
    const onLeft = mount()
    await confirmLeaving()
    expect((await screen.findByRole('alert')).textContent).toContain('The last manager cannot leave')
    expect(onLeft).not.toHaveBeenCalled()
  })

  it('offers to choose a new manager when the last one tries to leave', async () => {
    api.leaveChannel.mockRejectedValue(new ApiError(409, 'last_manager', 'the last manager cannot leave'))
    const choose = vi.fn()
    mount(vi.fn(), choose)
    await confirmLeaving()
    fireEvent.click(await screen.findByRole('button', { name: 'Choose a new manager' }))
    expect(choose).toHaveBeenCalledTimes(1)
  })

  it('does not offer it for any other failure', async () => {
    api.leaveChannel.mockRejectedValue(new ApiError(500, 'internal', 'boom'))
    mount(vi.fn(), vi.fn())
    await confirmLeaving()
    await screen.findByRole('alert')
    expect(screen.queryByRole('button', { name: 'Choose a new manager' })).toBeNull()
  })
})
