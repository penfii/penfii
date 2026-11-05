import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ createTeam: vi.fn(), searchDirectory: vi.fn() }))
vi.mock('../api', () => ({ api }))

import { CreateProvider, useCreate } from './create'
import { renderWithProviders } from '../test/renderWithProviders'

function Opener() {
  const open = useCreate()
  return <button type="button" onClick={() => open('team')}>open</button>
}

const person = (id: string, displayName: string) => ({ type: 'person', person: { id, displayName, username: null, email: '', organizationId: 'o', organizationName: 'O', affiliation: [] } })

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.searchDirectory.mockResolvedValue({ results: [person('u1', 'Mina'), person('u2', 'Noah')] })
  api.createTeam.mockResolvedValue({ id: 't', workspaceId: 'w', name: 'Core', archivedAt: null })
  renderWithProviders(<CreateProvider workspaceId="w" space={undefined}><Opener /></CreateProvider>, { route: '/' })
})
afterEach(cleanup)

describe('Creating a Team', () => {
  it('needs a first member and sends each member with the role chosen for them', async () => {
    fireEvent.click(screen.getByRole('button', { name: 'open' }))
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Core' } })
    const create = screen.getByRole('button', { name: 'Create team' }) as HTMLButtonElement
    expect(create.disabled).toBe(true)
    fireEvent.click(await screen.findByRole('button', { name: /Mina/ }))
    fireEvent.click(await screen.findByRole('button', { name: /Noah/ }))
    // Members alone are not enough: a Team needs a manager. Nobody is added for the person creating the Team.
    expect(create.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('Mina: Role'), { target: { value: 'MANAGER' } })
    expect(create.disabled).toBe(false)
    fireEvent.click(create)
    await waitFor(() => expect(api.createTeam).toHaveBeenCalledWith('w', {
      name: 'Core',
      members: [{ userId: 'u1', role: 'MANAGER' }, { userId: 'u2', role: 'MEMBER' }],
    }))
  })

  it('sends the optional description when one is written', async () => {
    fireEvent.click(screen.getByRole('button', { name: 'open' }))
    fireEvent.change(await screen.findByLabelText('Name'), { target: { value: 'Core' } })
    fireEvent.change(screen.getByLabelText('About (optional)'), { target: { value: '  Builds the core  ' } })
    fireEvent.click(await screen.findByRole('button', { name: /Mina/ }))
    fireEvent.change(screen.getByLabelText('Mina: Role'), { target: { value: 'MANAGER' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create team' }))
    await waitFor(() => expect(api.createTeam).toHaveBeenCalledWith('w', {
      name: 'Core',
      description: 'Builds the core',
      members: [{ userId: 'u1', role: 'MANAGER' }],
    }))
  })
})
