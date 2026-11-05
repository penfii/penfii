import { cleanup, render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { I18nProvider } from '../../i18n/I18nProvider'
import { InviteJoined } from './InviteJoined'

function at(search: string) {
  return render(
    <I18nProvider>
      <MemoryRouter initialEntries={[`/invite/joined${search}`]}><InviteJoined /></MemoryRouter>
    </I18nProvider>,
  )
}

beforeEach(() => localStorage.setItem('penfii.locale', 'en-US'))
afterEach(cleanup)

describe('landing after accepting through a provider', () => {
  it('says the Team was joined, with the name from the address', () => {
    at('?teamOutcome=JOINED&teamName=Core%20%26%20Co')
    expect(screen.getByText(/joined the Core & Co Team as a member/)).toBeTruthy()
  })
  it.each(['ARCHIVED', 'GONE'])('says the Team was not added: %s', (status) => {
    at(`?teamOutcome=${status}&teamName=Core`)
    expect(screen.getByText(/archived or no longer exists, so you were not added/)).toBeTruthy()
    expect(screen.getByText(/You joined\./)).toBeTruthy()
  })
  it('shows only the joined message when no Team was named, and ignores an unknown outcome', () => {
    at('?teamOutcome=bogus')
    expect(screen.getByText(/You joined\./)).toBeTruthy()
    expect(screen.queryByRole('status')).toBeNull()
  })
})
