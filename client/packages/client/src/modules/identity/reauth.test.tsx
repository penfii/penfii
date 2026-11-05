import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'

const api = vi.hoisted(() => ({ reauthPassword: vi.fn(), reauthProviderStart: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { useReauth } from './reauth'

function Probe({ action, onDone }: { action: () => Promise<string>; onDone: (value: string | undefined) => void }) {
  const { run, dialog } = useReauth()
  return (
    <div>
      <button type="button" onClick={() => void run(action).then(onDone)}>go</button>
      {dialog}
    </div>
  )
}

function mount(action: () => Promise<string>, onDone: (value: string | undefined) => void) {
  return render(
    <I18nProvider>
      <Probe action={action} onDone={onDone} />
    </I18nProvider>,
  )
}

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('reauthentication dialog', () => {
  it('asks once, proves the person, then retries the action', async () => {
    const action = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new ApiError(403, 'reauth_required', 'recent authentication is required', { methods: ['password'] }))
      .mockResolvedValueOnce('done')
    api.reauthPassword.mockResolvedValue(undefined)
    const onDone = vi.fn()
    mount(action, onDone)
    fireEvent.click(screen.getByText('go'))
    const input = await screen.findByLabelText('Password')
    fireEvent.change(input, { target: { value: 'secret' } })
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    await waitFor(() => expect(onDone).toHaveBeenCalledWith('done'))
    expect(api.reauthPassword).toHaveBeenCalledWith('secret')
    expect(action).toHaveBeenCalledTimes(2)
  })

  it('keeps the dialog open on a wrong password and does not retry', async () => {
    const action = vi
      .fn<() => Promise<string>>()
      .mockRejectedValue(new ApiError(403, 'reauth_required', 'x', { methods: ['password'] }))
    api.reauthPassword.mockRejectedValue(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    mount(action, vi.fn())
    fireEvent.click(screen.getByText('go'))
    fireEvent.change(await screen.findByLabelText('Password'), { target: { value: 'nope' } })
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    expect(await screen.findByText('The password is incorrect')).toBeTruthy()
    expect(action).toHaveBeenCalledTimes(1)
  })

  it('cancelling resolves without running the action again; other errors pass through', async () => {
    const action = vi
      .fn<() => Promise<string>>()
      .mockRejectedValue(new ApiError(403, 'reauth_required', 'x', { methods: ['password', 'provider:google'] }))
    const onDone = vi.fn()
    mount(action, onDone)
    fireEvent.click(screen.getByText('go'))
    expect(await screen.findByRole('button', { name: 'Continue with google' })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(onDone).toHaveBeenCalledWith(undefined))
    expect(action).toHaveBeenCalledTimes(1)
  })
})
