import { beforeEach, describe, expect, it } from 'vitest'
import type { FavoriteItem, Project } from '@penfii/contracts'
import { browseProjects, myProjects, readRecentProjects, recordRecentProject, switchTarget, switcherSections } from './projectsModel'

const project = (id: string, name: string, extra: Partial<Project> = {}): Project => ({ id, name, teamIds: [], participating: true, ...extra })
const star = (targetId: string, position: number, kind: FavoriteItem['kind'] = 'PROJECT'): FavoriteItem => ({ kind, targetId, position, createdAt: '', available: true, title: null, subtitle: null, projectId: null, teamId: null, viewTarget: null, layout: null }) as unknown as FavoriteItem

const projects = [project('a', 'Alpha'), project('b', 'Beta'), project('c', 'Gamma'), project('x', 'Outside', { participating: false })]

describe('My projects', () => {
  it('lists participating Projects only, starred first in favorite order, once each', () => {
    const mine = myProjects(projects, [star('c', 2), star('a', 1), star('a', 3), star('x', 0), star('zzz', 4), star('b', 5, 'TEAM')])
    expect(mine.favorites.map((item) => item.id)).toEqual(['a', 'c'])
    expect(mine.others.map((item) => item.id)).toEqual(['b'])
  })

  it('keeps every starred Project however many there are', () => {
    const many = Array.from({ length: 12 }, (_, index) => project(`p${index}`, `P${index}`))
    const mine = myProjects(many, many.map((item, index) => star(item.id, index)))
    expect(mine.favorites).toHaveLength(12)
    expect(mine.others).toHaveLength(0)
  })
})

describe('All projects', () => {
  it('filters by participation and by name or prefix, apart from what the person may see', () => {
    const withPrefix = [project('a', 'Alpha', { ticketPrefix: 'ALP' }), project('o', 'Other', { participating: false, ticketPrefix: 'OTH' })]
    expect(browseProjects(withPrefix, 'all', '').map((item) => item.id)).toEqual(['a', 'o'])
    expect(browseProjects(withPrefix, 'in', '').map((item) => item.id)).toEqual(['a'])
    expect(browseProjects(withPrefix, 'out', '').map((item) => item.id)).toEqual(['o'])
    expect(browseProjects(withPrefix, 'all', 'oth').map((item) => item.id)).toEqual(['o'])
    expect(browseProjects(withPrefix, 'all', '  ALP ').map((item) => item.id)).toEqual(['a'])
  })
})

describe('Project switcher', () => {
  beforeEach(() => localStorage.clear())

  it('orders favorites, recent visits, then the rest without repeating a Project, and never stars a recent one', () => {
    const sections = switcherSections(projects, [star('c', 1)], ['b', 'c', 'x', 'gone', 'a'], '')
    expect(sections.favorites.map((item) => item.id)).toEqual(['c'])
    // c is already a favorite; x is not a participating Project; "gone" no longer exists.
    expect(sections.recent.map((item) => item.id)).toEqual(['b', 'a'])
    expect(sections.others).toEqual([])
  })

  it('searches names and prefixes only inside the participating Projects, so an outside Project is never offered', () => {
    expect(switcherSections(projects, [], [], 'outside')).toEqual({ favorites: [], recent: [], others: [] })
    expect(switcherSections(projects, [], [], 'ALP').others.map((item) => item.id)).toEqual(['a'])
  })

  it('keeps the recent list per Workspace and person, newest first, without duplicates', () => {
    recordRecentProject('w', 'u1', 'a')
    recordRecentProject('w', 'u1', 'b')
    recordRecentProject('w', 'u1', 'a')
    expect(readRecentProjects('w', 'u1')).toEqual(['a', 'b'])
    expect(readRecentProjects('w', 'u2')).toEqual([])
    expect(readRecentProjects('w2', 'u1')).toEqual([])
  })

  it('goes to the same kind of page when it is usable there, never carrying an id of the old Project', () => {
    const target = project('t', 'Target')
    expect(switchTarget('w', target, 'work')).toBe('/w/w/projects/t/work')
    expect(switchTarget('w', target, 'timeline')).toBe('/w/w/projects/t/timeline')
    expect(switchTarget('w', target, 'overview')).toBe('/w/w/projects/t')
    // Settings only where the person administers the target; otherwise a safe overview.
    expect(switchTarget('w', target, 'settings')).toBe('/w/w/projects/t')
    expect(switchTarget('w', { ...target, canManage: true }, 'settings')).toBe('/w/w/projects/t/settings')
  })
})
