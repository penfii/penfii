import { Link, useParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { DraftItem } from '@penfii/contracts'
import { api } from '../../api'
import { EmptyState } from '../../design/ui'
import { formatMessageTime } from '../../format'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { CloseIcon } from '../../design/icons'

function draftLink(workspaceId: string, draft: DraftItem): string {
  const base = draft.projectId ? `/w/${workspaceId}/projects/${draft.projectId}/chat/${draft.conversationId}` : `/w/${workspaceId}/c/${draft.conversationId}`
  return draft.threadRootId ? `${base}/t/${draft.threadRootId}` : base
}

/** Drafts: messages started and not sent, from every compose box, in one place. They are the person's own. */
export function DraftsPage() {
  const { t, locale } = useI18n()
  const { workspaceId = '' } = useParams()
  const queryClient = useQueryClient()
  const drafts = useQuery({ ...queries.drafts(workspaceId), enabled: workspaceId.length > 0 })
  async function remove(id: string) {
    try {
      await api.deleteDraft(id)
    } finally {
      await queryClient.invalidateQueries({ queryKey: queryKeys.drafts(workspaceId) })
    }
  }
  return (
    <div className="page-scroll">
      <header className="page-intro">
        <h1>{t('drafts.title')}</h1>
        <p>{t('drafts.lead')}</p>
      </header>
      {drafts.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {drafts.isError ? <p className="board-error" role="alert">{t('saved.error')}</p> : null}
      {drafts.data && drafts.data.length === 0 ? <EmptyState title={t('drafts.empty')} body={t('drafts.emptyBody')} /> : null}
      <ul className="result-list saved-list">
        {(drafts.data ?? []).map((draft) => (
          <li key={draft.id} className="result-hit saved-row">
            <Link to={draftLink(workspaceId, draft)} className="saved-open">
              <small className="where-chip">{draft.threadRootId ? t('drafts.thread') : draft.title ? `# ${draft.title}` : t('drafts.dm')}</small> <strong>{draft.preview}</strong>
              <small className="hint"> · {formatMessageTime(draft.updatedAt, locale)}</small>
            </Link>
            <span className="saved-actions">
              <button type="button" className="icon-button" aria-label={t('drafts.delete')} onClick={() => void remove(draft.id)}><CloseIcon /></button>
            </span>
          </li>
        ))}
      </ul>
    </div>
  )
}
