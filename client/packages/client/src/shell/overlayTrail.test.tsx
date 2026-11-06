import { act, cleanup, render } from '@testing-library/react'
import { RouterProvider, createMemoryRouter, useNavigate } from 'react-router'
import { afterEach, describe, expect, it } from 'vitest'
import { DESKTOP, PHONE, setViewport } from '../test/viewport'
import { useLeaveOverlay, useTrackTrail } from './overlayTrail'

let leave: (to: string) => void = () => undefined
let go: ReturnType<typeof useNavigate> = (() => undefined) as never

function Probe() {
  useTrackTrail()
  leave = useLeaveOverlay()
  go = useNavigate()
  return null
}

function mount(entries: string[]) {
  const router = createMemoryRouter([{ path: '*', element: <Probe /> }], { initialEntries: entries, initialIndex: entries.length - 1 })
  render(<RouterProvider router={router} />)
  return router
}

afterEach(cleanup)

describe('closing a detail', () => {
  it('on a phone steps back when the detail was opened by a push from there, so Back never reaches it again', async () => {
    setViewport(PHONE)
    const router = mount(['/start', '/list'])
    await act(async () => { await go('/list?panel=ticket:1') })
    await act(async () => { leave('/list') })
    expect(router.state.location.pathname + router.state.location.search).toBe('/list')
    await act(async () => { await router.navigate(-1) })
    expect(router.state.location.pathname).toBe('/start')
  })

  it('on a phone replaces the entry when the detail was not opened from where it closes to', async () => {
    setViewport(PHONE)
    const router = mount(['/start', '/list?panel=ticket:1'])
    await act(async () => { leave('/list') })
    expect(router.state.location.pathname).toBe('/list')
    await act(async () => { await router.navigate(-1) })
    expect(router.state.location.pathname).toBe('/start')
  })

  it('on a wide window keeps the plain navigation: the closed place is added to the history', async () => {
    setViewport(DESKTOP)
    const router = mount(['/start', '/list'])
    await act(async () => { await go('/list?panel=ticket:1') })
    await act(async () => { leave('/list') })
    await act(async () => { await router.navigate(-1) })
    expect(router.state.location.search).toBe('?panel=ticket:1')
  })
})
