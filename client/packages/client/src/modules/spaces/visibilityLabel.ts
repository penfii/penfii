import type { MessageKey } from '../../i18n/messages'

type Translate = (key: MessageKey, params?: Record<string, string>) => string

/**
 * What a channel's visibility means to the person reading it. A Project's public channel is not public
 * to the Workspace: only the Project's members read and write it, so it is named for them.
 */
export function visibilityLabel(t: Translate, visibility: 'PUBLIC' | 'PRIVATE', inProject: boolean): string {
  if (visibility === 'PRIVATE') return t('channel.private')
  return inProject ? t('channel.projectPublic') : t('channel.public')
}

export function visibilityHint(t: Translate, inProject: boolean): string {
  return inProject ? t('channel.hint.project') : t('channel.hint.workspace')
}

/**
 * A public channel of a Project narrowed to one of its Teams is read by the people in both, not by the whole
 * Project, so it is named "<Team> of Project <Project>" instead of "Project members".
 */
export function channelScopeLabel(t: Translate, visibility: 'PUBLIC' | 'PRIVATE', projectName: string | undefined, teamName: string | undefined): string {
  if (visibility === 'PUBLIC' && projectName && teamName) return t('docs.scope.projectTeam', { project: projectName, team: teamName })
  return visibilityLabel(t, visibility, projectName !== undefined)
}
