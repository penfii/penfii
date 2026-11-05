import { describe, expect, it } from 'vitest'
import { canChangeChannel, canCreateChannel, canDeleteMessage, canDetachAttachment, canInviteToChannel, canLeaveChannel, canSetChannelRoles, isWorkspaceManager } from './capabilities'

describe('capabilities', () => {
  it('offers a Workspace-wide channel to the Workspace owner or admin only', () => {
    expect(['OWNER', 'ADMIN', 'MEMBER', undefined].map((role) => canCreateChannel(role))).toEqual([true, true, false, false])
    expect(isWorkspaceManager('MEMBER')).toBe(false)
  })

  it('offers a Project channel to the Project admin, whatever the Workspace role', () => {
    expect(canCreateChannel('MEMBER', { canManage: true })).toBe(true)
    expect(canCreateChannel('OWNER', { canManage: false })).toBe(false)
    expect(canCreateChannel('ADMIN', {})).toBe(false)
  })

  it('lets the channel manager, the Project admin or a Workspace admin of a public global channel change a channel', () => {
    expect(canChangeChannel({ role: 'MANAGER', visibility: 'PRIVATE' }, 'MEMBER')).toBe(true)
    expect(canChangeChannel({ projectId: 'p', visibility: 'PUBLIC' }, 'MEMBER', { canManage: true })).toBe(true)
    expect(canChangeChannel({ projectId: 'p', visibility: 'PUBLIC' }, 'ADMIN', { canManage: false })).toBe(false)
    expect(canChangeChannel({ visibility: 'PUBLIC' }, 'ADMIN')).toBe(true)
    expect(canChangeChannel({ visibility: 'PRIVATE' }, 'OWNER')).toBe(false)
    expect(canChangeChannel({ visibility: 'PUBLIC' }, 'MEMBER')).toBe(false)
  })

  it('lets the author or the channel manager delete a message', () => {
    expect(canDeleteMessage({ authorId: 'a' }, 'a')).toBe(true)
    expect(canDeleteMessage({ authorId: 'a' }, 'b', 'MANAGER')).toBe(true)
    expect(canDeleteMessage({ authorId: 'a' }, 'b', null)).toBe(false)
  })

  it('lets the attacher or the Project admin remove an attachment', () => {
    expect(canDetachAttachment({ attachedBy: 'a' }, 'a', false)).toBe(true)
    expect(canDetachAttachment({ attachedBy: 'a' }, 'b', true)).toBe(true)
    expect(canDetachAttachment({ attachedBy: 'a' }, 'b', false)).toBe(false)
  })

  it('offers putting people into a private channel to its manager, its Project admin, and members when the channel allows it', () => {
    const base = { visibility: 'PRIVATE', projectId: null as string | null }
    expect(canInviteToChannel({ ...base, role: 'MANAGER' })).toBe(true)
    expect(canInviteToChannel({ ...base, role: 'MEMBER' })).toBe(false)
    expect(canInviteToChannel({ ...base, role: 'MEMBER', allowMemberInvites: true })).toBe(true)
    expect(canInviteToChannel({ ...base, role: null, allowMemberInvites: true })).toBe(false)
    expect(canInviteToChannel({ ...base, projectId: 'p', role: 'MEMBER' }, { canManage: true })).toBe(true)
    expect(canInviteToChannel({ ...base, projectId: 'p', role: 'MEMBER' }, { canManage: false })).toBe(false)
  })

  it('lets the channel manager and the Project admin give roles in a private channel, and nobody else', () => {
    const base = { visibility: 'PRIVATE', projectId: null as string | null }
    expect(canSetChannelRoles({ ...base, role: 'MANAGER' })).toBe(true)
    expect(canSetChannelRoles({ ...base, role: 'MEMBER' })).toBe(false)
    expect(canSetChannelRoles({ ...base, role: 'MEMBER' }, { canManage: true }), 'only in a Project channel').toBe(false)
    expect(canSetChannelRoles({ ...base, projectId: 'p', role: 'MEMBER' }, { canManage: true })).toBe(true)
    expect(canSetChannelRoles({ ...base, projectId: 'p', role: 'MEMBER' }, { canManage: false })).toBe(false)
    expect(canSetChannelRoles({ visibility: 'PUBLIC', role: 'MANAGER' })).toBe(false)
  })

  it('lets a member of a private channel leave it, and nobody else', () => {
    expect(canLeaveChannel({ visibility: 'PRIVATE', role: 'MEMBER' })).toBe(true)
    expect(canLeaveChannel({ visibility: 'PRIVATE', role: 'MANAGER' })).toBe(true)
    expect(canLeaveChannel({ visibility: 'PRIVATE', role: null })).toBe(false)
    expect(canLeaveChannel({ visibility: 'PUBLIC', role: 'MEMBER' }), 'public channels are not left here').toBe(false)
  })

  it('does not offer it for a public channel, which people join themselves', () => {
    expect(canInviteToChannel({ visibility: 'PUBLIC', role: 'MANAGER' })).toBe(false)
  })
})
