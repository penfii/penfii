import { cleanup, fireEvent, screen } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Space } from '@penfii/contracts'

const api = vi.hoisted(() => ({ getSpace: vi.fn() }))
vi.mock('../../api', () => ({ api }))

import { ShareScopePicker, scopeLabel, type ShareScope } from './ShareScopePicker'
import { renderWithProviders } from '../../test/renderWithProviders'

const space = {
  teams: [{ id: 't-design', name: 'Design' }, { id: 't-mkt', name: 'Marketing' }, { id: 't-other', name: 'Design (B)' }],
  projects: [
    { id: 'pa', name: 'Alpha', teamIds: ['t-design', 't-mkt'] },
    { id: 'pb', name: 'Beta', teamIds: ['t-other'] },
  ],
  channels: [],
  dms: [],
} as unknown as Space

function Harness({ onValue }: { onValue: (v: ShareScope) => void }) {
  const [value, setValue] = useState<ShareScope>({ visibility: 'WORKSPACE', projectId: null, teamId: null })
  return <ShareScopePicker workspaceId="w" name="s" value={value} onChange={(v) => { setValue(v); onValue(v) }} suggestedProjectId="pa" />
}

beforeEach(() => {
  localStorage.setItem('penfii.locale', 'ko-KR')
  api.getSpace.mockResolvedValue(space)
})
afterEach(cleanup)

describe('share scope', () => {
  it('names the scope the way people read it', () => {
    const t = (key: string, params?: Record<string, string>) => ({ 'docs.scope.workspace': 'W', 'docs.scope.private': 'P', 'docs.scope.project': 'Project', 'docs.scope.projectNamed': 'Project {project}', 'docs.scope.projectTeam': '{team} of Project {project}' } as Record<string, string>)[key]?.replace(/\{(\w+)\}/g, (_, k: string) => params?.[k] ?? '') ?? key
    expect(scopeLabel({ visibility: 'WORKSPACE', projectId: null, teamId: null }, space, t as never)).toBe('W')
    expect(scopeLabel({ visibility: 'RESTRICTED', projectId: null, teamId: null }, space, t as never)).toBe('P')
    expect(scopeLabel({ visibility: 'PROJECT', projectId: 'pa', teamId: null }, space, t as never)).toBe('Project Alpha')
    expect(scopeLabel({ visibility: 'PROJECT', projectId: 'pa', teamId: 't-design' }, space, t as never)).toBe('Design of Project Alpha')
  })

  it('offers a Project, then only that Project\'s Teams, and never a Team on its own', async () => {
    const seen: ShareScope[] = []
    renderWithProviders(<Harness onValue={(v) => seen.push(v)} />)
    const project = await screen.findByLabelText('프로젝트')
    expect((project as HTMLInputElement).type).toBe('radio')
    fireEvent.click(project)
    // The Project of the place the page is made in comes first; the Team list is that Project's own.
    const team = (await screen.findAllByRole('combobox')).at(-1) as HTMLSelectElement
    expect(seen.at(-1)).toEqual({ visibility: 'PROJECT', projectId: 'pa', teamId: null })
    expect([...team.options].map((o) => o.textContent)).toEqual(['프로젝트 전체 (팀 제한 없음)', 'Design', 'Marketing'])
    fireEvent.change(team, { target: { value: 't-design' } })
    expect(seen.at(-1)).toEqual({ visibility: 'PROJECT', projectId: 'pa', teamId: 't-design' })
    // Another Project resets the Team: a Team of one Project does not carry over.
    fireEvent.change((screen.getAllByRole('combobox')[0]) as HTMLSelectElement, { target: { value: 'pb' } })
    expect(seen.at(-1)).toEqual({ visibility: 'PROJECT', projectId: 'pb', teamId: null })
    fireEvent.click(screen.getByLabelText('초대한 사람만 (비공개)'))
    expect(seen.at(-1)).toEqual({ visibility: 'RESTRICTED', projectId: null, teamId: null })
  })

  it('lists only the Projects the person takes part in', async () => {
    api.getSpace.mockResolvedValue({ ...space, projects: [{ id: 'pa', name: 'Alpha', teamIds: [], participating: true }, { id: 'pb', name: 'Beta', teamIds: [], participating: false }] })
    renderWithProviders(<Harness onValue={() => undefined} />)
    fireEvent.click(await screen.findByLabelText('프로젝트'))
    const select = (await screen.findAllByRole('combobox'))[0] as HTMLSelectElement
    expect([...select.options].map((o) => o.textContent)).toEqual(['Alpha'])
  })
})
