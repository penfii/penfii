import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { ApiError } from '@penfii/api-client'
import { RouterProvider, createMemoryRouter } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ me: vi.fn(), login: vi.fn(), authOptions: vi.fn(), serverInfo: vi.fn(), sessions: vi.fn() }))
vi.mock('../api', () => ({ api }))

import { LoginPage } from './LoginPage'
import { I18nProvider } from '../i18n/I18nProvider'
import { peekReturn, rememberReturn } from '../app/returnTo'
import { queryKeys } from '../queryKeys'

const dana = { id: 'u-dana', email: 'dana@example.com', displayName: 'Dana' }
const unauthenticated = () => new ApiError(401, 'unauthenticated', 'unauthenticated')

function mount(entries: string[] = ['/login'], client = new QueryClient({ defaultOptions: { queries: { retry: false } } })) {
  const router = createMemoryRouter(
    [
      { path: '/login', element: <LoginPage /> },
      { path: '/', element: <p>home</p> },
      { path: '/w/:workspaceId/docs', element: <p>docs</p> },
      { path: '/w/:workspaceId/projects/:projectId', element: <p>project</p> },
      { path: '*', element: <p>elsewhere</p> },
    ],
    { initialEntries: entries, initialIndex: entries.length - 1 },
  )
  const view = render(
    <QueryClientProvider client={client}>
      <I18nProvider>
        <RouterProvider router={router} />
      </I18nProvider>
    </QueryClientProvider>,
  )
  return { router, client, ...view }
}

const form = () => screen.queryByLabelText('Email')

beforeEach(() => {
  localStorage.clear()
  sessionStorage.clear()
  localStorage.setItem('penfii.locale', 'en-US')
  api.authOptions.mockResolvedValue({ providers: [], longSession: { enabled: false, maxAgeSeconds: 0, idleSeconds: 0 } })
  api.serverInfo.mockResolvedValue({ features: [], limits: {} })
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the sign-in page when a session may already exist (FEA-195)', () => {
  it('waits for the server without showing the form, then replaces itself with the start page for a valid session', async () => {
    let answer: (value: unknown) => void = () => undefined
    api.me.mockImplementation(() => new Promise((resolve) => { answer = resolve }))
    const { router } = mount(['/start', '/login'])
    expect(screen.getByRole('status').textContent).toBe('Loading')
    expect(form()).toBeNull()
    await act(async () => answer(dana))
    expect(await screen.findByText('home')).toBeTruthy()
    expect(form()).toBeNull()
    expect(router.state.location.pathname).toBe('/')
    // Back does not return to the sign-in page: it was replaced.
    await act(async () => { await router.navigate(-1) })
    expect(router.state.location.pathname).toBe('/start')
  })

  it('goes to the place that was remembered for this person, once', async () => {
    api.me.mockResolvedValue(dana)
    sessionStorage.setItem('penfii.afterLogin', JSON.stringify({ path: '/w/w1/docs?doc=d1', userId: 'u-dana' }))
    const { router } = mount()
    expect(await screen.findByText('docs')).toBeTruthy()
    expect(router.state.location.search).toBe('?doc=d1')
    expect(peekReturn()).toBeUndefined()
  })

  it('a place remembered for someone else, or for nobody in particular, is not leaked to this person', async () => {
    api.me.mockResolvedValue(dana)
    sessionStorage.setItem('penfii.afterLogin', JSON.stringify({ path: '/w/w1/projects/secret', userId: 'u-erin' }))
    mount()
    expect(await screen.findByText('home')).toBeTruthy()
    expect(peekReturn()).toBeUndefined()
    cleanup()
    // An invitation link is remembered for nobody: it is kept for whoever signs in next.
    sessionStorage.setItem('penfii.afterLogin', JSON.stringify({ path: '/w/w1/docs', userId: null }))
    mount()
    expect(await screen.findByText('docs')).toBeTruthy()
  })

  it.each(['//evil.example/path', '/\\evil.example', 'https://evil.example/', 'javascript:alert(1)', '/login', '/login?x=1', '/w/w1/docs\n/x'])(
    'never goes to %j',
    async (bad) => {
      api.me.mockResolvedValue(dana)
      sessionStorage.setItem('penfii.afterLogin', JSON.stringify({ path: bad, userId: 'u-dana' }))
      const { router } = mount()
      expect(await screen.findByText('home')).toBeTruthy()
      expect(router.state.location.pathname).toBe('/')
    },
  )

  it('shows the form when there is no session, or it has expired or been revoked', async () => {
    api.me.mockRejectedValue(unauthenticated())
    mount()
    expect(await screen.findByLabelText('Email')).toBeTruthy()
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('does not take a person cached in this tab as an answer: the server says the session is over', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    client.setQueryData(queryKeys.me, dana)
    api.me.mockRejectedValue(unauthenticated())
    const { router } = mount(['/login'], client)
    // The cached person is not enough to leave, nor to show the form before the server has answered.
    expect(form()).toBeNull()
    expect(await screen.findByLabelText('Email')).toBeTruthy()
    expect(router.state.location.pathname).toBe('/login')
    expect(api.me).toHaveBeenCalledTimes(1)
  })

  it('does not call a server or network failure a failed sign-in: it says so and asks again on request', async () => {
    api.me.mockRejectedValueOnce(new ApiError(503, 'db_unavailable', 'the database is temporarily unavailable'))
    mount()
    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toMatch(/database is temporarily unavailable/i)
    expect(form()).toBeNull()
    expect(screen.queryByText(/incorrect/i)).toBeNull()
    api.me.mockResolvedValueOnce(dana)
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }))
    expect(await screen.findByText('home')).toBeTruthy()
    expect(api.me).toHaveBeenCalledTimes(2)
    // A network error is the same.
    cleanup()
    api.me.mockRejectedValueOnce(new TypeError('Failed to fetch'))
    mount()
    expect((await screen.findByRole('alert')).textContent).toMatch(/could not be completed/i)
    expect(form()).toBeNull()
  })

  it('leaves a provider sign-in that came back with its message where it is, even for a valid session', async () => {
    api.me.mockResolvedValue(dana)
    // The form reads the message from the address bar, as in the browser.
    window.history.replaceState({}, '', '/login?auth_error=not_linked')
    try {
      const { router } = mount(['/login?auth_error=not_linked'])
      expect(await screen.findByLabelText('Email')).toBeTruthy()
      expect(screen.getByText(/No user is linked to that account/)).toBeTruthy()
      expect(router.state.location.pathname).toBe('/login')
    } finally {
      window.history.replaceState({}, '', '/')
    }
  })

  it('does not take the person on before the form does: a sign-in that works lands on the remembered place', async () => {
    api.me.mockRejectedValueOnce(unauthenticated())
    api.login.mockImplementation(async () => {
      // From now on the server knows the person, as it does the moment the cookie is set.
      api.me.mockResolvedValue(dana)
    })
    sessionStorage.setItem('penfii.afterLogin', JSON.stringify({ path: '/w/w1/docs', userId: null }))
    const { router } = mount()
    fireEvent.change(await screen.findByLabelText('Email'), { target: { value: 'dana@example.com' } })
    fireEvent.change(document.querySelector('input[type=password]')!, { target: { value: 'right-password' } })
    fireEvent.click(screen.getByRole('button', { name: /^Log in/ }))
    await waitFor(() => expect(router.state.location.pathname).toBe('/w/w1/docs'))
    // Settled: not carried on to the start page afterwards.
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 50)) })
    expect(router.state.location.pathname).toBe('/w/w1/docs')
    expect(api.login).toHaveBeenCalledTimes(1)
  })

  it('a remembered place is kept for the form when the person is not signed in', async () => {
    api.me.mockRejectedValue(unauthenticated())
    rememberReturn('/w/w1/docs')
    mount()
    expect(await screen.findByLabelText('Email')).toBeTruthy()
    expect(peekReturn()).toBe('/w/w1/docs')
  })
})
