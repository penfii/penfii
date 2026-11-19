import { useCallback, useLayoutEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { isApiError } from '@penfii/api-client'
import type { DocNode } from '@penfii/contracts'
import { api } from '../../api'
import { Button, Dialog } from '../../design/ui'
import { notify } from '../../design/notices'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { deleteScope, scopeKey, type DeleteScope } from './treeModel'

/** How many deleted subpages are listed before "and N more". */
const LISTED = 10

type Phase =
  | { kind: 'checking' }
  | { kind: 'ready'; scope: DeleteScope; changed: boolean }
  | { kind: 'checkFailed'; forbidden: boolean }
  | { kind: 'deleting'; scope: DeleteScope }
  | { kind: 'failed'; scope: DeleteScope; forbidden: boolean }

export type Deleted = { removedIds: ReadonlySet<string> }

/**
 * The question before a document is deleted. A deleted document cannot be brought back in this product, so the dialog
 * says exactly what goes (and what stays, moved up) before it enables Delete; if that cannot be found out, nothing is deleted.
 */
export function DeleteDocDialog({
  workspaceId,
  node,
  onClose,
  onDeleted,
}: {
  workspaceId: string
  node: DocNode | null
  onClose: () => void
  /** The pages that are really gone (read again after the delete), or null when that could not be read. */
  onDeleted: (node: DocNode, result: Deleted | null) => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [phase, setPhase] = useState<Phase>({ kind: 'checking' })
  const [listOpen, setListOpen] = useState(false)
  // The newest question wins: a slow answer to an earlier one (another page, or a retry) is ignored.
  const round = useRef(0)

  /**
   * The whole Workspace's tree, read now: never from the cache (it may be stale) and never into it (a failed read must not turn the
   * tree behind this dialog into an error). A Project's tree leaves out subpages that belong to other Projects, so it is not used.
   */
  const readTree = useCallback(() => api.getDocTree(workspaceId), [workspaceId])

  // The tree behind re-renders while the question is open; the question itself only restarts for another page.
  const closeRef = useRef(onClose)
  closeRef.current = onClose
  const gone = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) })
    notify({ kind: 'info', message: t('docs.del.gone') })
    closeRef.current()
  }, [queryClient, workspaceId, t])

  const check = useCallback(async () => {
    if (!node) return
    const mine = ++round.current
    setPhase({ kind: 'checking' })
    setListOpen(false)
    try {
      const scope = deleteScope(await readTree(), node.id)
      if (mine !== round.current) return
      if (!scope) gone()
      else setPhase({ kind: 'ready', scope, changed: false })
    } catch (error) {
      if (mine === round.current) setPhase({ kind: 'checkFailed', forbidden: isApiError(error) && error.status === 403 })
    }
  }, [node, readTree, gone])

  // Before paint, so a question reopened for another page never shows the last one's answer.
  const nodeId = node?.id
  const checkRef = useRef(check)
  checkRef.current = check
  useLayoutEffect(() => {
    if (nodeId) void checkRef.current()
    else round.current += 1
  }, [nodeId])

  async function remove(known: DeleteScope) {
    if (!node) return
    const mine = ++round.current
    setPhase({ kind: 'deleting', scope: known })
    try {
      // Read once more: the structure may have changed while the question was open. Only an unchanged scope is deleted.
      const before = await readTree()
      const fresh = deleteScope(before, node.id)
      if (mine !== round.current) return
      if (!fresh) {
        gone()
        return
      }
      if (scopeKey(fresh) !== scopeKey(known)) {
        setPhase({ kind: 'ready', scope: fresh, changed: true })
        return
      }
      await api.deleteDoc(node.id)
      let result: Deleted | null = null
      try {
        const after = await readTree()
        const remaining = new Set(after.map((item) => item.id))
        result = { removedIds: new Set(before.filter((item) => !remaining.has(item.id)).map((item) => item.id)) }
      } catch {
        result = null
      }
      void queryClient.invalidateQueries({ queryKey: queryKeys.all.docTree(workspaceId) })
      announce(known, result)
      onDeleted(node, result)
    } catch (error) {
      if (mine !== round.current) return
      if (isApiError(error) && error.status === 404) gone()
      else setPhase({ kind: 'failed', scope: known, forbidden: isApiError(error) && error.status === 403 })
    }
  }

  function announce(expected: DeleteScope, result: Deleted | null) {
    const title = expected.deleted[0].title || t('docs.untitled')
    if (!result) {
      notify({ message: t('docs.del.done.plain') })
      return
    }
    const count = result.removedIds.size
    const expectedIds = new Set(expected.deleted.map((item) => item.id))
    const same = count === expectedIds.size && [...result.removedIds].every((id) => expectedIds.has(id))
    const text = count <= 1 ? t('docs.del.done.one', { title }) : t('docs.del.done.many', { title, count: String(count - 1) })
    notify({ message: same ? text : `${text} ${t('docs.del.done.different', { count: String(count) })}` })
  }

  const scope = phase.kind === 'ready' || phase.kind === 'deleting' || phase.kind === 'failed' ? phase.scope : null
  const deleting = phase.kind === 'deleting'
  return (
    <Dialog open={node !== null} title={t('docs.delete.title')} onClose={() => { if (!deleting) onClose() }}>
      <div className="doc-delete" aria-busy={phase.kind === 'checking' || deleting}>
        {phase.kind === 'checking' ? <p role="status">{t('docs.del.checking')}</p> : null}
        {phase.kind === 'checkFailed' ? (
          <p role="alert">
            <strong>{t('docs.del.checkFailed')}</strong>
            <br />
            {phase.forbidden ? t('docs.del.checkForbidden') : t('docs.del.checkFailedWhy')}
          </p>
        ) : null}
        {scope ? <Scope scope={scope} changed={phase.kind === 'ready' && phase.changed} open={listOpen} onToggle={() => setListOpen((current) => !current)} /> : null}
        {phase.kind === 'failed' ? (
          <p role="alert" className="form-error">{phase.forbidden ? t('docs.del.forbidden') : t('docs.del.failed')}</p>
        ) : null}
        {deleting ? <p role="status">{t('docs.del.deleting')}</p> : null}
        <p className="hint">{t('docs.del.noRestore')}</p>
      </div>
      <div className="dialog-actions">
        <Button variant="secondary" data-autofocus disabled={deleting} onClick={onClose}>{t('docs.cancel')}</Button>
        {phase.kind === 'checkFailed' && !phase.forbidden ? (
          <Button variant="secondary" onClick={() => void check()}>{t('docs.del.recheck')}</Button>
        ) : null}
        {phase.kind === 'failed' && !phase.forbidden ? <Button variant="danger" onClick={() => void remove(phase.scope)}>{t('docs.del.retry')}</Button> : null}
        {phase.kind === 'ready' || deleting ? (
          <Button variant="danger" disabled={deleting} onClick={() => phase.kind === 'ready' && void remove(phase.scope)}>{t('docs.delete')}</Button>
        ) : (
          <Button variant="danger" disabled>{t('docs.delete')}</Button>
        )}
      </div>
    </Dialog>
  )
}

function Scope({ scope, changed, open, onToggle }: { scope: DeleteScope; changed: boolean; open: boolean; onToggle: () => void }) {
  const { t } = useI18n()
  const title = scope.deleted[0].title || t('docs.untitled')
  const below = scope.deleted.slice(1)
  const names = (items: readonly DocNode[]) => items.map((item) => `“${item.title || t('docs.untitled')}”`)
  const movedNames = names(scope.moved.slice(0, 3)).join(', ') + (scope.moved.length > 3 ? ` ${t('docs.del.more', { count: String(scope.moved.length - 3) })}` : '')
  return (
    <>
      {changed ? <p role="alert" className="form-error">{t('docs.del.changed')}</p> : null}
      <p className="confirm-body">{below.length === 0 ? t('docs.del.one', { title }) : t('docs.del.many', { title, count: String(below.length) })}</p>
      {below.length > 0 ? (
        <>
          <button type="button" className="link-button" aria-expanded={open} onClick={onToggle}>{t('docs.del.show', { count: String(below.length) })}</button>
          {open ? (
            <ul className="doc-delete-list">
              {below.slice(0, LISTED).map((item) => <li key={item.id}>{item.title || t('docs.untitled')}</li>)}
              {below.length > LISTED ? <li>{t('docs.del.more', { count: String(below.length - LISTED) })}</li> : null}
            </ul>
          ) : null}
        </>
      ) : null}
      {scope.moved.length > 0 ? (
        <p className="confirm-body">
          {scope.destination ? t('docs.del.moved', { titles: movedNames, parent: scope.destination.title || t('docs.untitled') }) : t('docs.del.movedTop', { titles: movedNames })}
        </p>
      ) : null}
    </>
  )
}
