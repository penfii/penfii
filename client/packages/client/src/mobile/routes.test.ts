import { describe, expect, it } from 'vitest'
import { pushTarget, resolveScreen, rootPath } from './routes'

const W = 'w1'
const screen = (section: string, path: string, search = '') => resolveScreen(W, section, `/w/${W}${path}`, search)

describe('resolveScreen: roots', () => {
  it('treats each tab root as a root', () => {
    for (const [section, path] of [['home', ''], ['inbox', '/inbox'], ['chat', '/chat'], ['projects', '/projects'], ['docs', '/docs'], ['teams', '/directory'], ['search', '/search'], ['settings', '/settings']] as const) {
      expect(screen(section, path).root, path).toBe(true)
    }
  })

  it('does not take a query that opens content for a root', () => {
    expect(screen('inbox', '/inbox', '?box=mentions').root).toBe(false)
    expect(screen('docs', '/docs', '?doc=d1').root).toBe(false)
    expect(screen('docs', '/docs', '?draft=1').root).toBe(false)
    expect(screen('search', '/search', '?tab=all&q=alpha').root).toBe(false)
    expect(screen('settings', '/settings', '?screen=profile').root).toBe(false)
    expect(screen('projects', '/projects/p1/work', '?panel=ticket%3At1').root).toBe(false)
  })

  it('keeps a query that does not open content on the root', () => {
    expect(screen('search', '/search', '?tab=ticket').root).toBe(true)
    expect(screen('docs', '/docs', '?edit=1').root).toBe(true)
  })

  it('treats anything below a root path as a push', () => {
    expect(screen('projects', '/projects/p1').root).toBe(false)
    expect(screen('chat', '/c/c1').root).toBe(false)
    expect(screen('teams', '/directory/members').root).toBe(false)
    expect(screen('settings', '/settings/account').root).toBe(false)
    expect(screen('inbox', '/saved').root).toBe(false)
  })

  it('ignores a trailing slash', () => {
    expect(screen('projects', '/projects/').root).toBe(true)
  })
})

describe('resolveScreen: area and title', () => {
  it('keeps Chat lit for a conversation and Settings for the account page', () => {
    expect(screen('projects', '/c/c1').area).toBe('chat')
    expect(screen('account', '/settings/account').area).toBe('settings')
  })

  it('names the screen', () => {
    expect(screen('inbox', '/saved').title).toEqual({ kind: 'key', key: 'mobile.vault' })
    expect(screen('inbox', '/drafts').vault).toBe(true)
    expect(screen('projects', '/projects/p1/work').title).toEqual({ kind: 'project', id: 'p1' })
    expect(screen('projects', '/projects/p1/chat/c9').title).toEqual({ kind: 'conversation', id: 'c9' })
    expect(screen('chat', '/c/c2/t/m1').title).toEqual({ kind: 'conversation', id: 'c2' })
    expect(screen('docs', '/docs', '?doc=d1').title).toEqual({ kind: 'doc', id: 'd1', projectId: null })
    expect(screen('projects', '/projects/p1/docs', '?doc=d1').title).toEqual({ kind: 'doc', id: 'd1', projectId: 'p1' })
    expect(screen('projects', '/projects/p1/planning', '?sprintDetail=s1').title).toEqual({ kind: 'sprint', id: 's1', projectId: 'p1' })
    expect(screen('settings', '/settings/notifications').title).toEqual({ kind: 'key', key: 'settings.notifications' })
    expect(screen('settings', '/settings', '?screen=profile').title).toEqual({ kind: 'key', key: 'settings.profile' })
  })

  it('has no name for a project root list', () => {
    expect(screen('projects', '/projects').title).toBeNull()
  })
})

describe('pushTarget and rootPath', () => {
  it('marks a link that would land on the root again', () => {
    expect(pushTarget(W, 'teams', { pathname: `/w/${W}/directory`, search: '' })).toBe(`/w/${W}/directory?screen=1`)
  })

  it('leaves a link that already opens content alone', () => {
    expect(pushTarget(W, 'inbox', { pathname: `/w/${W}/inbox`, search: '?box=mentions' })).toBe(`/w/${W}/inbox?box=mentions`)
    expect(pushTarget(W, 'projects', { pathname: `/w/${W}/projects/p1`, search: '' })).toBe(`/w/${W}/projects/p1`)
    expect(pushTarget(W, 'home', { pathname: `/w/${W}/inbox`, search: '' })).toBe(`/w/${W}/inbox`)
  })

  it('gives the root of an area', () => {
    expect(rootPath(W, 'teams')).toBe(`/w/${W}/directory`)
    expect(rootPath(W, 'home')).toBe(`/w/${W}`)
  })
})
