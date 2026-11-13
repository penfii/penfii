import { describe, expect, it } from 'vitest'
import { isBareUrlText, parseInternalLink } from './internalLink'

const O = 'https://app.penfii.test'
const parse = (path: string) => parseInternalLink(`${O}${path}`, O)

describe('parseInternalLink', () => {
  it('finds channels, threads, project chat, docs, tickets and projects', () => {
    expect(parse('/w/w1/c/c1')).toEqual({ kind: 'channel', workspaceId: 'w1', conversationId: 'c1', threadRootId: null })
    expect(parse('/w/w1/c/c1/t/m9')).toEqual({ kind: 'thread', workspaceId: 'w1', conversationId: 'c1', threadRootId: 'm9' })
    expect(parse('/w/w1/projects/p1/chat/c2')).toEqual({ kind: 'channel', workspaceId: 'w1', conversationId: 'c2', threadRootId: null })
    expect(parse('/w/w1/projects/p1/chat/c2/t/m3')).toEqual({ kind: 'thread', workspaceId: 'w1', conversationId: 'c2', threadRootId: 'm3' })
    expect(parse('/w/w1/docs?doc=d1')).toEqual({ kind: 'doc', workspaceId: 'w1', docId: 'd1' })
    expect(parse('/w/w1/projects/p1/work?panel=ticket:t1')).toEqual({ kind: 'ticket', workspaceId: 'w1', projectId: 'p1', ticketId: 't1' })
    expect(parse('/w/w1/projects/p1')).toEqual({ kind: 'project', workspaceId: 'w1', projectId: 'p1' })
    expect(parse('/w/w1/c/c1/')).toEqual({ kind: 'channel', workspaceId: 'w1', conversationId: 'c1', threadRootId: null })
  })

  it('leaves other origins, other sites and pages that are not targets as plain links', () => {
    expect(parseInternalLink('https://other.test/w/w1/c/c1', O)).toBeNull()
    expect(parseInternalLink('https://example.com', O)).toBeNull()
    expect(parse('/w/w1/docs')).toBeNull()
    expect(parse('/w/w1/docs?doc=new')).toBeNull()
    expect(parse('/w/w1/projects/p1/work')).toBeNull()
    expect(parse('/w/w1/settings')).toBeNull()
    expect(parse('/login')).toBeNull()
    expect(parseInternalLink('not a url', O)).toBeNull()
  })

  it('tells a bare URL from the words a person wrote', () => {
    expect(isBareUrlText(`${O}/w/w1/c/c1`, `${O}/w/w1/c/c1`)).toBe(true)
    expect(isBareUrlText('see the channel', `${O}/w/w1/c/c1`)).toBe(false)
  })
})
