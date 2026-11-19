import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { queryKeys } from '../queryKeys'

const api = vi.hoisted(() => ({ me: vi.fn(), updateProfile: vi.fn(), listWorkspaces: vi.fn(), listPeople: vi.fn() }))
vi.mock('../api', () => ({ api }))

import { ProfileSettings } from './ProfileSettings'
import { renderWithProviders } from '../test/renderWithProviders'

const me = (patch: Record<string, unknown> = {}) => ({
  id: 'u1', email: 'a@x', displayName: 'Alice', locale: null, timeZone: 'Asia/Seoul', jobTitle: null, statusEmoji: null, statusText: null, statusClearAt: null, hasAvatar: false, ...patch,
})

let client: ReturnType<typeof renderWithProviders>['client']
async function mount() {
  const mounted = renderWithProviders(<ProfileSettings workspaceId="w" />, { route: '/w/w/settings/profile' })
  client = mounted.client
  return (await screen.findByLabelText(/^Display name/)) as HTMLInputElement
}
/** The server's profile changes elsewhere (the avatar menu, another device) and this tab reads it. */
async function serverChanges(patch: Record<string, unknown>) {
  api.me.mockResolvedValue(me(patch))
  await act(async () => { await client.invalidateQueries({ queryKey: queryKeys.me }) })
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  Object.values(api).forEach((fn) => fn.mockReset())
  api.me.mockResolvedValue(me())
  api.listWorkspaces.mockResolvedValue([{ id: 'w', name: 'W', role: 'MEMBER' }])
  api.listPeople.mockResolvedValue([])
  api.updateProfile.mockResolvedValue(me())
})
afterEach(cleanup)

describe('Profile: unsaved changes', () => {
  it('shows no bar until something changes, then counts the changed items and returns to nothing when typed back', async () => {
    const name = await mount()
    expect(screen.queryByRole('button', { name: 'Save' })).toBeNull()
    fireEvent.change(name, { target: { value: 'Alicia' } })
    expect(screen.getByText('Unsaved changes · 1 items')).toBeTruthy()
    expect(screen.getByText('Not saved')).toBeTruthy()
    fireEvent.change(screen.getByLabelText(/^Title/), { target: { value: 'Dev' } })
    expect(screen.getByText('Unsaved changes · 2 items')).toBeTruthy()
    fireEvent.change(screen.getByLabelText(/^Title/), { target: { value: '' } })
    fireEvent.change(name, { target: { value: 'Alice ' } })
    expect(screen.queryByText(/Unsaved changes/)).toBeNull()
    expect(screen.queryByRole('button', { name: 'Save' })).toBeNull()
  })

  it('reverts everything without asking', async () => {
    const name = await mount()
    fireEvent.change(name, { target: { value: 'Alicia' } })
    fireEvent.click(screen.getByRole('button', { name: 'Clear status' }))
    fireEvent.click(screen.getByRole('button', { name: 'Revert' }))
    expect(name.value).toBe('Alice')
    expect(screen.queryByText(/Unsaved changes/)).toBeNull()
  })

  it('saves, says when, and is unsaved again at the next keystroke', async () => {
    const name = await mount()
    fireEvent.change(name, { target: { value: 'Alicia' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.updateProfile).toHaveBeenCalledTimes(1))
    expect(api.updateProfile).toHaveBeenCalledWith({ displayName: 'Alicia', jobTitle: null, timeZone: 'Asia/Seoul', statusEmoji: null, statusText: null, statusClearAt: null })
    expect((await screen.findByText(/^Saved ·/)).getAttribute('role')).toBe('status')
    expect(screen.queryByRole('button', { name: 'Save' })).toBeNull()
    fireEvent.change(name, { target: { value: 'Alicia K' } })
    expect(screen.getByText('Unsaved changes · 1 items')).toBeTruthy()
  })

  it('does not send a blank name and says why under the field', async () => {
    const name = await mount()
    fireEvent.change(name, { target: { value: '   ' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    const alert = await screen.findByText('Enter a display name. A name of only spaces cannot be saved.')
    expect(api.updateProfile).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(name)
    expect(name.getAttribute('aria-describedby')).toBe(alert.id)
  })

  it('keeps the input and offers Save again when saving fails; a refused name is shown under the field', async () => {
    const name = await mount()
    fireEvent.change(name, { target: { value: 'Alicia' } })
    api.updateProfile.mockRejectedValueOnce(new ApiError(502, 'error', 'HTTP 502'))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(await screen.findByText(/Could not save the profile/)).toBeTruthy()
    expect(name.value).toBe('Alicia')
    expect(screen.getByText('Unsaved changes · 1 items')).toBeTruthy()
    api.updateProfile.mockRejectedValueOnce(new ApiError(400, 'validation', 'name is invalid'))
    fireEvent.click(screen.getByRole('button', { name: 'Save again' }))
    expect(await screen.findByText(/This display name cannot be used/)).toBeTruthy()
    expect(screen.queryByText('name is invalid')).toBeNull()
  })
})

describe('Profile: the server changes while editing', () => {
  it('keeps the typed name and takes the new status the person did not touch', async () => {
    const name = await mount()
    fireEvent.change(name, { target: { value: 'Alicia' } })
    await serverChanges({ statusEmoji: '🍱', statusText: 'Lunch' })
    await waitFor(() => expect((screen.getByLabelText('Status text') as HTMLInputElement).value).toBe('Lunch'))
    expect(name.value).toBe('Alicia')
    expect(screen.getByText('Unsaved changes · 1 items')).toBeTruthy()
  })

  it('shows both values for the same item and lets the person choose', async () => {
    await mount()
    fireEvent.change(screen.getByLabelText('Status text'), { target: { value: 'Mine' } })
    await serverChanges({ statusEmoji: '🍱', statusText: 'Lunch' })
    const group = await screen.findByRole('group', { name: 'Differs from the value saved elsewhere' })
    expect(within(group).getByText('Mine')).toBeTruthy()
    expect(within(group).getByText('🍱 Lunch')).toBeTruthy()
    expect((screen.getByLabelText('Status text') as HTMLInputElement).value).toBe('Mine')
    fireEvent.click(within(group).getByRole('button', { name: 'Use latest value' }))
    expect((screen.getByLabelText('Status text') as HTMLInputElement).value).toBe('Lunch')
    expect(screen.queryByRole('group', { name: 'Differs from the value saved elsewhere' })).toBeNull()
    expect(screen.queryByText(/Unsaved changes/)).toBeNull()
  })

  it('does not save while an item conflicts: the read just before saving finds it', async () => {
    await mount()
    fireEvent.change(screen.getByLabelText('Status text'), { target: { value: 'Mine' } })
    // The other device saves, but this tab has not heard yet: only the read inside Save finds out.
    api.me.mockResolvedValue(me({ statusEmoji: '🍱', statusText: 'Lunch' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(await screen.findByText('Not saved. Something changed elsewhere.')).toBeTruthy()
    expect(api.updateProfile).not.toHaveBeenCalled()
    const group = await screen.findByRole('group', { name: 'Differs from the value saved elsewhere' })
    // "Keep my input" answers the question; saving then sends the input.
    fireEvent.click(within(group).getByRole('button', { name: 'Keep my input' }))
    fireEvent.click(screen.getAllByRole('button', { name: 'Save again' })[0])
    await waitFor(() => expect(api.updateProfile).toHaveBeenCalledWith(expect.objectContaining({ statusText: 'Mine' })))
  })
})

describe('Profile: edge cases', () => {
  it('keeps what is typed while the save reads the profile again, and leaves it unsaved afterwards', async () => {
    const name = await mount()
    fireEvent.change(name, { target: { value: 'Alicia' } })
    // The read just before saving is slow; the person keeps typing meanwhile.
    let answer: (value: unknown) => void = () => undefined
    api.me.mockReturnValueOnce(new Promise((resolve) => { answer = resolve }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(api.me).toHaveBeenCalledTimes(2))
    fireEvent.change(name, { target: { value: 'Alicia K' } })
    await act(async () => answer(me()))
    await waitFor(() => expect(api.updateProfile).toHaveBeenCalledTimes(1))
    expect(api.updateProfile).toHaveBeenCalledWith(expect.objectContaining({ displayName: 'Alicia' }))
    expect(name.value).toBe('Alicia K')
    expect(await screen.findByText('Unsaved changes · 1 items')).toBeTruthy()
  })

  it('says a tab in the job title is not accepted, under that field, and sends nothing', async () => {
    await mount()
    const title = screen.getByLabelText(/^Title/) as HTMLInputElement
    fireEvent.change(title, { target: { value: 'Lead\tDev' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    const alert = await screen.findByText('Tabs and control characters cannot be used here.')
    expect(api.updateProfile).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(title)
    expect(title.getAttribute('aria-describedby')).toBe(alert.id)
    expect(screen.queryByText(/display name/i, { selector: '.field-error' })).toBeNull()
  })
})
