import { cleanup, fireEvent, screen } from '@testing-library/react'
import { ApiError } from '@penfii/api-client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ login: vi.fn(), authProviders: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { LoginForm } from './LoginForm'
import { renderWithProviders } from '../../test/renderWithProviders'

function mount() {
  return renderWithProviders(<LoginForm />, { route: '/' })
}

async function submit(password: string) {
  fireEvent.change(screen.getByLabelText(/email/i), { target: { value: 'dana@example.com' } })
  fireEvent.change(document.querySelector('input[type=password]')!, { target: { value: password } })
  fireEvent.click(screen.getByRole('button', { name: 'Log in' }))
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.authProviders.mockResolvedValue([])
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('sign-in form', () => {
  it('applies no password rule: a short password is sent to the server as typed', async () => {
    api.login.mockRejectedValue(new ApiError(401, 'unauthenticated', 'unauthenticated'))
    mount()
    const input = document.querySelector('input[type=password]') as HTMLInputElement
    expect(input.minLength).toBe(-1)
    await submit('1234')
    expect(await screen.findByText(/incorrect/i)).toBeTruthy()
    expect(api.login).toHaveBeenCalledWith({ email: 'dana@example.com', password: '1234' })
  })

  it('says it is a lock, and for how long, when the server answers 429', async () => {
    api.login.mockRejectedValue(new ApiError(429, 'rate_limited', 'too many attempts, try again later', { retryAfterSeconds: 610 }))
    mount()
    await submit('whatever')
    expect(await screen.findByText('Too many attempts. Try again in 11 minutes')).toBeTruthy()
  })

  it('falls back to 15 minutes when the server gives no time', async () => {
    api.login.mockRejectedValue(new ApiError(429, 'rate_limited', 'x'))
    mount()
    await submit('whatever')
    expect(await screen.findByText('Too many attempts. Try again in 15 minutes')).toBeTruthy()
  })
})
