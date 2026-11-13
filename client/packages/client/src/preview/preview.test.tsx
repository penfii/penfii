import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ getSpace: vi.fn(), getProjectBoard: vi.fn(), getProjectMembers: vi.fn() }))
vi.mock('../api', () => ({ api }))

import { I18nProvider } from '../i18n/I18nProvider'
import { loadPreview } from './load'
import { previewSlots } from './slots'

const originals = { ...api }
const realProject = { id: 'real', name: 'Real', teamIds: [], participating: true }

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'en-US')
  api.getSpace.mockResolvedValue({ teams: [], projects: [realProject], channels: [], dms: [] })
  api.getProjectBoard.mockResolvedValue({ projectId: 'real', columns: [], states: [] })
  api.getProjectMembers.mockResolvedValue([{ userId: 'u' }])
})
afterEach(() => {
  cleanup()
  Object.assign(api, originals)
  delete previewSlots.overview
  vi.unstubAllEnvs()
})

describe('a real workspace has no sample content', () => {
  it('leaves the app as it is unless a demo build asks', async () => {
    await loadPreview()
    expect(api.getSpace).toBe(originals.getSpace)
    expect(previewSlots.overview).toBeUndefined()
    expect((await api.getSpace('w')).projects.map((project: { id: string }) => project.id)).toEqual(['real'])
  })
})

describe('a demo build keeps its sample content', () => {
  async function demo() {
    vi.stubEnv('VITE_PREVIEW_CONTENT', '1')
    await loadPreview()
  }

  it('lists the sample Projects before the real ones, as ordinary Projects', async () => {
    await demo()
    const projects = (await api.getSpace('w')).projects as Array<{ id: string; participating?: boolean }>
    expect(projects.map((project) => project.id)).toEqual(['penfii', 'fds', 'platform', 'real'])
    expect(projects.every((project) => project.participating)).toBe(true)
  })

  it('answers what a sample Project header reads without asking the server, and passes real Projects on', async () => {
    await demo()
    expect(await api.getProjectBoard('fds')).toMatchObject({ projectId: 'fds', columns: [] })
    expect(await api.getProjectMembers('fds')).toEqual([])
    expect(originals.getProjectBoard).not.toHaveBeenCalled()
    await api.getProjectBoard('real')
    expect(originals.getProjectBoard).toHaveBeenCalledWith('real')
  })

  it('draws an overview for a sample Project only', async () => {
    await demo()
    expect(previewSlots.overview?.('other')).toBeNull()
    render(<I18nProvider>{previewSlots.overview?.('penfii')}</I18nProvider>)
    expect(screen.getByText('42%')).toBeTruthy()
  })

  it('installs once, however often it is asked', async () => {
    await demo()
    await demo()
    expect(((await api.getSpace('w')).projects as unknown[]).length).toBe(4)
  })
})
