import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router'
import { EmptyState } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'

/**
 * A conversation that cannot be opened: it was deleted, the person lost it, or it never was. The server says the same for
 * all three (and for a private channel the person is not in), so the page says no more than that and shows the way back:
 * to the Project's chat when the conversation was opened from a Project the person can still open, else to Chat.
 */
export function ConversationUnavailable({ workspaceId, projectId }: { workspaceId: string; projectId?: string }) {
  const { t } = useI18n()
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  // The Project's chat only while the Space says the person can open the Project: a return target that is itself closed would
  // be another dead end.
  const project = projectId ? space.data?.projects.find((item) => item.id === projectId) : undefined
  const toProject = project !== undefined && project.participating !== false
  return (
    <div className="page-scroll" data-state="conversation-unavailable">
      <EmptyState title={t('conversation.unavailable.title')} body={t('conversation.unavailable.body')}>
        <Link className="btn btn-secondary" to={toProject ? `/w/${workspaceId}/projects/${project.id}/chat` : `/w/${workspaceId}/chat`}>
          {toProject ? t('conversation.unavailable.toProjectChat') : t('conversation.unavailable.toChat')}
        </Link>
      </EmptyState>
    </div>
  )
}

/** A thread that cannot be opened while its conversation can: the way back is the conversation, with the thread closed. */
export function ThreadUnavailable({ conversationHref }: { conversationHref: string }) {
  const { t } = useI18n()
  return (
    <div className="thread-unavailable" data-state="thread-unavailable">
      <EmptyState title={t('thread.unavailable.title')} body={t('thread.unavailable.body')}>
        <Link className="btn btn-secondary" to={conversationHref}>{t('thread.unavailable.back')}</Link>
      </EmptyState>
    </div>
  )
}
