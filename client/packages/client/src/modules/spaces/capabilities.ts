/**
 * Which actions a person is offered, in the server's terms (`View` / `Collaborate` / `Manage` plus the
 * role or ownership a rule names). The server decides every call; these only keep a button from being
 * shown that would answer 403, so each one mirrors a server rule.
 */

/** A Workspace role as the Workspace list carries it. */
export function isWorkspaceManager(role: string | undefined): boolean {
  return role === 'OWNER' || role === 'ADMIN'
}

/**
 * Creating a channel: a Project's channel belongs to that Project's admin; a Workspace-wide channel to
 * the Workspace owner or admin. (The server counts a Project admin from a direct or a group grant.)
 */
export function canCreateChannel(workspaceRole: string | undefined, project?: { canManage?: boolean } | null): boolean {
  if (project) return project.canManage === true
  return isWorkspaceManager(workspaceRole)
}

/**
 * Changing a channel's visibility or description: the channel's MANAGER, the admin of its Project, or a
 * Workspace owner/admin for a public Workspace-wide channel.
 */
export function canChangeChannel(
  channel: { role?: string | null; projectId?: string | null; visibility: string },
  workspaceRole: string | undefined,
  project?: { canManage?: boolean } | null,
): boolean {
  if (channel.role === 'MANAGER') return true
  if (channel.projectId) return project?.canManage === true
  return channel.visibility !== 'PRIVATE' && isWorkspaceManager(workspaceRole)
}

/**
 * Giving a member of a private channel the manager role, or taking it back: the channel's MANAGER and
 * the admin of its Project, the same people who manage who is in it. The server keeps the last manager.
 */
export function canSetChannelRoles(
  channel: { role?: string | null; projectId?: string | null; visibility: string },
  project?: { canManage?: boolean } | null,
): boolean {
  if (channel.visibility !== 'PRIVATE') return false
  return channel.role === 'MANAGER' || Boolean(channel.projectId && project?.canManage === true)
}

/** A member of a private channel may leave it. The server keeps the last manager in. */
export function canLeaveChannel(channel: { role?: string | null; visibility: string }): boolean {
  return channel.visibility === 'PRIVATE' && Boolean(channel.role)
}

/**
 * Putting people into a channel, which is how anyone but its creator reaches a private one. The server
 * allows it to the channel's MANAGER, the admin of its Project, and any member when the channel allows
 * member invitations; a public channel needs none of it, people join it themselves.
 */
export function canInviteToChannel(
  channel: { role?: string | null; projectId?: string | null; visibility: string; allowMemberInvites?: boolean },
  project?: { canManage?: boolean } | null,
): boolean {
  if (channel.visibility !== 'PRIVATE') return false
  if (channel.role === 'MANAGER') return true
  if (channel.projectId && project?.canManage === true) return true
  return Boolean(channel.role) && channel.allowMemberInvites === true
}

/** Deleting a message: its author, or the channel's MANAGER. Editing stays with the author. */
export function canDeleteMessage(message: { authorId: string }, currentUserId: string, channelRole?: string | null): boolean {
  return message.authorId === currentUserId || channelRole === 'MANAGER'
}

/** Removing a file from a Ticket's body: the person who attached it, or the Project admin. */
export function canDetachAttachment(attachment: { attachedBy: string }, currentUserId: string, canManageProject: boolean): boolean {
  return attachment.attachedBy === currentUserId || canManageProject
}
