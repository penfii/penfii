import { lazy, Suspense, useState } from 'react'
import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import type { DocPage, DocVersionSummary } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Dialog } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import type { DocEditorContext } from './docContext'

const DocEditor = lazy(() => import('./DocEditor'))

/**
 * History: the Versions people created, newest first. A Version can be viewed, and restored into the
 * editor as a draft; nothing here changes the page or the history.
 */
export function DocHistoryDialog({
  open,
  page,
  workspaceId,
  onClose,
  onRestore,
  openReference,
}: {
  open: boolean
  page: DocPage
  workspaceId: string
  onClose: () => void
  /** Hands the chosen Version to the page, which opens the editor on it. Absent when the person cannot edit. */
  onRestore?: (versionNo: number) => void
  openReference: DocEditorContext['open']
}) {
  const { t, locale } = useI18n()
  const [viewing, setViewing] = useState<number | null>(null)
  const list = useInfiniteQuery({
    queryKey: queryKeys.docVersions(page.id),
    queryFn: ({ pageParam }) => api.getDocVersions(page.id, pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.nextBefore ?? undefined,
    enabled: open,
  })
  const items: DocVersionSummary[] = list.data?.pages.flatMap((p) => p.items) ?? []
  const when = (iso: string) => new Date(iso).toLocaleString(locale)

  function close() {
    setViewing(null)
    onClose()
  }

  return (
    <Dialog open={open} title={t('docs.history.title')} onClose={close}>
      {viewing !== null ? (
        <VersionPreview
          page={page}
          workspaceId={workspaceId}
          versionNo={viewing}
          onBack={() => setViewing(null)}
          onRestore={onRestore ? () => { setViewing(null); onRestore(viewing) } : undefined}
          openReference={openReference}
        />
      ) : list.isPending ? (
        <p className="hint" role="status">{t('loading')}</p>
      ) : list.isError ? (
        <p className="board-error" role="alert">{t('docs.history.loadError')}</p>
      ) : items.length === 0 ? (
        <p className="hint">{t('docs.history.empty')}</p>
      ) : (
        <>
          {onRestore ? <p className="hint">{t('docs.history.restoreHint')}</p> : null}
          <ul className="doc-history">
            {items.map((item) => (
              <li key={item.versionNo} className="doc-history-row">
                <div>
                  <strong>{t('docs.history.item', { no: String(item.versionNo) })}</strong>
                  {' · '}
                  {item.title}
                  {item.isCurrent ? <span className="doc-badge">{t('docs.history.current')}</span> : null}
                  <div className="hint">{item.createdByName} · {when(item.createdAt)}</div>
                </div>
                <span className="doc-history-actions">
                  <Button variant="secondary" onClick={() => setViewing(item.versionNo)}>{t('docs.history.view')}</Button>
                  {onRestore ? <Button variant="secondary" onClick={() => { onRestore(item.versionNo) }}>{t('docs.history.restore')}</Button> : null}
                </span>
              </li>
            ))}
          </ul>
          {list.hasNextPage ? (
            <Button variant="ghost" disabled={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()}>{t('docs.history.more')}</Button>
          ) : null}
        </>
      )}
      <div className="dialog-actions">
        <Button variant="ghost" onClick={close}>{t('docs.close')}</Button>
      </div>
    </Dialog>
  )
}

function VersionPreview({
  page,
  workspaceId,
  versionNo,
  onBack,
  onRestore,
  openReference,
}: {
  page: DocPage
  workspaceId: string
  versionNo: number
  onBack: () => void
  onRestore?: () => void
  openReference: DocEditorContext['open']
}) {
  const { t } = useI18n()
  const version = useQuery({ ...queries.docVersion(page.id, versionNo), retry: false })
  return (
    <div className="doc-history-preview">
      <div className="doc-head">
        <Button variant="ghost" onClick={onBack}>{t('docs.history.back')}</Button>
        <span style={{ flex: 1 }} />
        {onRestore ? <Button onClick={onRestore}>{t('docs.history.restore')}</Button> : null}
      </div>
      {version.isPending ? <p className="hint" role="status">{t('loading')}</p> : null}
      {version.isError ? <p className="board-error" role="alert">{t('docs.history.loadError')}</p> : null}
      {version.data ? (
        <>
          <h3>{version.data.title}</h3>
          <Suspense fallback={<p className="hint" role="status">{t('loading')}</p>}>
            <DocEditor
              key={`${page.id}:v${versionNo}`}
              docId={page.id}
              projectId={page.projectId}
              workspaceId={workspaceId}
              content={version.data.content}
              editable={false}
              open={openReference}
            />
          </Suspense>
        </>
      ) : null}
    </div>
  )
}
