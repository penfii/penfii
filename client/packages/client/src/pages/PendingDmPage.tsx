import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Navigate, useLocation, useNavigate, useParams, useSearchParams } from 'react-router'
import { api } from '../api'
import { useI18n } from '../i18n/I18nProvider'
import { newIdempotencyKey } from '@penfii/api-client'
import { Composer } from '../modules/conversations'
import { useSession } from '../modules/identity'
import { queryKeys } from '../queryKeys'
import { queries } from '../queries'
import { EMPTY_RICH_TEXT, isBlankDoc, sameRichText } from '../richtext'
import { readPeers, startedConversation, type PendingPeer } from '../shell/pendingDm'
import { errorText } from '../ui/errorText'
import { NotFoundState } from '../ui/NotFoundState'

export function PendingDmPage() {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const [params] = useSearchParams()
  const location = useLocation()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const session = useSession()
  const ids = (params.get('users') ?? '').split(',').filter((id) => id.length > 0)
  const statePeers = (location.state as { peers?: PendingPeer[] } | null)?.peers
  const peers = (statePeers?.length ? statePeers : readPeers(ids)).filter((peer) => ids.includes(peer.id))
  const space = useQuery({ ...queries.space(workspaceId), enabled: workspaceId.length > 0 })
  const serverInfo = useQuery(queries.serverInfo())
  const [doc, setDoc] = useState(EMPTY_RICH_TEXT)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  if (session.isPending) return <p className="hint">{t('loading')}</p>
  if (!session.data || ids.length === 0) return <NotFoundState />
  const memberIds = [session.data.id, ...ids]
  const started = startedConversation(space.data?.dms ?? [], memberIds)
  if (started) return <Navigate to={`/w/${workspaceId}/c/${started.conversationId}`} replace />

  const title = peers.map((peer) => peer.displayName).join(', ') || t('dm.short')
  const maxBytes = serverInfo.data ? serverInfo.data.limits.messageBodyMaxBytes : undefined

  async function send(fileIds: string[]): Promise<boolean> {
    if (!session.data || busy) return false
    if (isBlankDoc(doc) && fileIds.length === 0) return false
    setBusy(true)
    setError(null)
    try {
      const dm = await api.createDm(workspaceId, { userIds: [session.data.id, ...ids] })
      await api.sendMessage(
        dm.conversationId,
        { bodySchemaVersion: 1, body: doc, threadRootId: null, replyToId: null, fileIds },
        newIdempotencyKey(),
      )
      setDoc((current) => (sameRichText(current, doc) ? EMPTY_RICH_TEXT : current))
      await queryClient.invalidateQueries({ queryKey: queryKeys.space(workspaceId) })
      void navigate(`/w/${workspaceId}/c/${dm.conversationId}`, { replace: true })
      return true
    } catch (caught) {
      setError(errorText(caught, t('message.failed')))
      return false
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="conversation-frame">
      <header className="context-bar">
        <div>
          <h1>{title}</h1>
          <p>{t('dm.pending')}</p>
        </div>
      </header>
      <div className="conversation no-thread">
        <div className="column">
          <section className="transcript">
            <div className="scroller">
              <p className="hint">{t('timeline.empty')}</p>
            </div>
          </section>
          {error ? <p className="form-error">{error}</p> : null}
          <Composer
            value={doc}
            onChange={setDoc}
            workspaceId={workspaceId}
            onSend={send}
            messageBodyMaxBytes={maxBytes}
            busy={busy}
            label={t('composer.placeholder')}
            focusToken="pending"
          />
        </div>
      </div>
    </div>
  )
}
