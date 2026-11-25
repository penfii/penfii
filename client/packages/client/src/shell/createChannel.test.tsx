import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '../queryKeys'

const api = vi.hoisted(() => ({ createChannel: vi.fn() }))
vi.mock('../api', () => ({ api }))

import { CreateProvider, useCreate } from './create'
import { renderWithProviders } from '../test/renderWithProviders'

function Opener() {
  const open = useCreate()
  return <button type="button" onClick={() => open('channel')}>open</button>
}

let client: ReturnType<typeof renderWithProviders>['client']

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.createChannel.mockReset()
  const mounted = renderWithProviders(<CreateProvider workspaceId="w" space={undefined}><Opener /></CreateProvider>, { route: '/' })
  client = mounted.client
})
afterEach(cleanup)

async function openDialog() {
  fireEvent.click(screen.getByRole('button', { name: 'open' }))
  return (await screen.findByLabelText('Channel name')) as HTMLInputElement
}
const submit = () => fireEvent.click(screen.getByRole('button', { name: 'Create channel' }))

describe('Creating a channel: the name', () => {
  it('says nothing before a submit or on blur, then explains a blank name below the field and focuses it', async () => {
    const name = await openDialog()
    fireEvent.focus(name)
    fireEvent.blur(name)
    expect(screen.queryByRole('alert')).toBeNull()
    fireEvent.change(name, { target: { value: '   ' } })
    submit()
    const alert = await screen.findByText('Enter a channel name. A name of only spaces cannot be used.')
    expect(api.createChannel).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(name)
    expect(name.getAttribute('aria-invalid')).toBe('true')
    expect(name.getAttribute('aria-describedby')).toBe(alert.id)
    // After the first submit every keystroke is checked again, so the error goes as soon as the name is valid.
    fireEvent.change(name, { target: { value: 'ops' } })
    expect(screen.queryByText(/Enter a channel name/)).toBeNull()
    expect(name.getAttribute('aria-invalid')).toBeNull()
  })

  it('limits the name to 200 characters', async () => {
    const name = await openDialog()
    expect(name.maxLength).toBe(200)
  })
})

describe('Creating a channel: one request', () => {
  it('sends one request however many times it is submitted, and cannot be closed meanwhile', async () => {
    let finish: (value: { conversationId: string }) => void = () => undefined
    api.createChannel.mockReturnValue(new Promise((resolve) => { finish = resolve }))
    const name = await openDialog()
    fireEvent.change(name, { target: { value: 'ops' } })
    submit()
    const busy = await screen.findByRole('button', { name: 'Creating…' })
    fireEvent.click(busy)
    fireEvent.submit(name.closest('form')!)
    expect((screen.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    fireEvent(screen.getByRole('dialog', { hidden: true }), new Event('cancel', { cancelable: true, bubbles: false }))
    expect(api.createChannel).toHaveBeenCalledTimes(1)
    expect(screen.getByLabelText('Channel name')).toBeTruthy()
    finish({ conversationId: 'c1' })
    await waitFor(() => expect(screen.queryByLabelText('Channel name')).toBeNull())
    expect(api.createChannel).toHaveBeenCalledTimes(1)
  })
})

describe('Creating a channel: what a failure says', () => {
  async function fail(error: unknown, value = 'ops') {
    api.createChannel.mockRejectedValue(error)
    const name = await openDialog()
    fireEvent.change(name, { target: { value } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'notes' } })
    fireEvent.change(screen.getByLabelText(/^Visibility/), { target: { value: 'PRIVATE' } })
    submit()
    await waitFor(() => expect(api.createChannel).toHaveBeenCalled())
    return name
  }
  const keepsInput = () => {
    expect((screen.getByLabelText('Channel name') as HTMLInputElement).value).toBe('ops')
    expect((screen.getByLabelText('Description') as HTMLInputElement).value).toBe('notes')
    expect((screen.getByLabelText(/^Visibility/) as HTMLSelectElement).value).toBe('PRIVATE')
  }

  it('shows a validation refusal under the name in the screen language, never the server text', async () => {
    const name = await fail(new ApiError(400, 'validation', 'name is invalid'))
    await screen.findByText('This name cannot be used. Use 200 characters or fewer, without line breaks or control characters.')
    expect(screen.queryByText('name is invalid')).toBeNull()
    expect(document.activeElement).toBe(name)
    keepsInput()
  })

  it.each([
    [new ApiError(0, 'offline', 'You are offline'), 'You are offline, so the request was not sent. Try again.'],
    [new ApiError(403, 'forbidden', 'forbidden'), 'You do not have permission to create this channel.'],
    [new ApiError(409, 'team_archived', 'archived'), 'The selected team is archived, so the channel cannot be created. Choose another team.'],
    [new ApiError(404, 'not_found', 'not found'), 'The workspace or project was not found. It may have been deleted.'],
  ])('explains %# above the form and keeps the input', async (error, text) => {
    await fail(error)
    await screen.findByText(text)
    keepsInput()
  })

  it.each([
    ['a network error', new TypeError('Failed to fetch')],
    ['a server error', new ApiError(502, 'error', 'HTTP 502')],
  ])('after %s says a retry of the same form is safe, and refreshes the channel list', async (_label, error) => {
    const invalidate = vi.spyOn(client, 'invalidateQueries')
    await fail(error)
    const text = await screen.findByText(/could not confirm whether the channel was created/)
    expect(text.textContent).toContain('Trying again with the same details is safe')
    expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.space('w') })
    keepsInput()
  })

  it('can be submitted again after a failure, with the same key for the same form', async () => {
    await fail(new TypeError('Failed to fetch'))
    api.createChannel.mockResolvedValue({ conversationId: 'c1' })
    submit()
    await waitFor(() => expect(api.createChannel).toHaveBeenCalledTimes(2))
    const [first, second] = api.createChannel.mock.calls
    expect(typeof first[2]).toBe('string')
    expect(second[2]).toBe(first[2])
  })

  it('uses a new key once the form changes, and for the next channel', async () => {
    const name = await fail(new TypeError('Failed to fetch'))
    fireEvent.change(name, { target: { value: 'ops-2' } })
    api.createChannel.mockResolvedValue({ conversationId: 'c1' })
    submit()
    await waitFor(() => expect(api.createChannel).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(screen.queryByLabelText('Channel name')).toBeNull())
    const again = await openDialog()
    fireEvent.change(again, { target: { value: 'ops-2' } })
    submit()
    await waitFor(() => expect(api.createChannel).toHaveBeenCalledTimes(3))
    const keys = api.createChannel.mock.calls.map((call) => call[2])
    expect(new Set(keys).size).toBe(3)
  })
})

describe('Creating a channel: the description', () => {
  it('explains a description the server would refuse under that field, not under the name, and sends nothing', async () => {
    const name = await openDialog()
    fireEvent.change(name, { target: { value: 'ops' } })
    const description = screen.getByLabelText('Description') as HTMLInputElement
    fireEvent.change(description, { target: { value: 'pasted\twith a tab' } })
    submit()
    const alert = await screen.findByText(/A description cannot contain tabs or control characters/)
    expect(api.createChannel).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(description)
    expect(description.getAttribute('aria-describedby')).toBe(alert.id)
    expect(screen.queryByText(/This name cannot be used/)).toBeNull()
    fireEvent.change(description, { target: { value: 'fine' } })
    expect(screen.queryByText(/A description cannot contain/)).toBeNull()
  })
})
