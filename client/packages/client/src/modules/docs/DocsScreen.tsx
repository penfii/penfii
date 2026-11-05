import { useCallback, useEffect, useState } from 'react'
import './knowledge.css'
import { useParams, useSearchParams } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { Button, EmptyState } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queries } from '../../queries'
import { DocPageView } from './DocPageView'
import { DocTree } from './DocTree'
import { DraftDoc } from './DraftDoc'
import { NewDocDialog } from './NewDocDialog'
import type { ShareScope } from './ShareScopePicker'
import { useDocTree, useDocTreeCommands } from './useDocs'

/** `/w/:ws/docs` (and the Project tab, filtered to that Project). The page is chosen by `?doc=`. */
export function DocsScreen({ projectId, withTree = false }: { projectId?: string; withTree?: boolean }) {
  const { t } = useI18n()
  const { workspaceId = '' } = useParams()
  const [params, setParams] = useSearchParams()
  const selected = params.get('doc')
  const drafting = params.get('draft') === '1'
  const draftParent = params.get('parent')
  const commands = useDocTreeCommands(workspaceId)
  const tree = useDocTree(workspaceId, projectId ?? null)
  const hasPages = (tree.data?.length ?? 0) > 0
  const templates = useQuery(queries.docTemplates(workspaceId))
  // A page just created opens in the editor (`edit=1`); leaving an unsaved draft is asked about by the page (UnsavedBlocker).
  const [creating, setCreating] = useState<{ templateId?: string; defaultTitle?: string } | null>(null)
  const select = (id: string | null, options?: { edit?: boolean }) => {
    const next = new URLSearchParams(params)
    next.delete('draft')
    next.delete('parent')
    if (id) next.set('doc', id)
    else next.delete('doc')
    if (id && options?.edit) next.set('edit', '1')
    else next.delete('edit')
    setParams(next)
  }
  const startDraft = (parentId: string | null) => {
    const next = new URLSearchParams(params)
    next.delete('doc')
    next.delete('edit')
    next.set('draft', '1')
    if (parentId) next.set('parent', parentId)
    else next.delete('parent')
    setParams(next)
  }
  const onEditStarted = useCallback(() => {
    setParams((current) => {
      const next = new URLSearchParams(current)
      next.delete('edit')
      return next
    }, { replace: true })
  }, [setParams])
  useEffect(() => {
    // `?doc=new` was the old placeholder; a page needs a title now, so it asks for one.
    if (selected === 'new') {
      setCreating({})
      const next = new URLSearchParams(params)
      next.delete('doc')
      setParams(next, { replace: true })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected])

  async function create(title: string, scope: ShareScope) {
    const page = await commands.create.mutateAsync({
      title,
      projectId: projectId ?? null,
      templateId: creating?.templateId,
      visibility: scope.visibility,
      scopeProjectId: scope.projectId,
      scopeTeamId: scope.teamId,
    })
    setCreating(null)
    select(page.id, { edit: true })
  }

  const content = drafting ? (
    <DraftDoc
      workspaceId={workspaceId}
      projectId={projectId ?? null}
      parentId={draftParent}
      onCreated={(id) => select(id, { edit: true })}
      onDiscard={() => select(null)}
    />
  ) : selected && selected !== 'new' ? (
    <DocPageView key={selected} workspaceId={workspaceId} docId={selected} startEditing={params.get('edit') === '1'} onEditStarted={onEditStarted} />
  ) : (
    <div className="doc-page">
      <EmptyState title={t('nav.docs')} body={hasPages ? t('docs.tree.pick') : t('docs.tree.empty')}>
        <Button onClick={() => startDraft(null)}>{t('docs.new')}</Button>
        {templates.data && templates.data.length > 0 ? (
          <p className="doc-templates">
            {templates.data.map((template) => (
              <Button
                key={template.id}
                variant="secondary"
                onClick={() => setCreating({ templateId: template.id, defaultTitle: template.name })}
              >
                {t('docs.template.use', { name: template.name })}
              </Button>
            ))}
          </p>
        ) : null}
      </EmptyState>
    </div>
  )
  const dialog = <NewDocDialog open={creating !== null} workspaceId={workspaceId} defaultTitle={creating?.defaultTitle} suggestedProjectId={projectId ?? null} onCreate={create} onClose={() => setCreating(null)} />
  if (!withTree) return <div className="page-scroll">{content}{dialog}</div>
  return (
    <div className="page-scroll doc-split">
      <aside className="doc-split-tree">
        <DocTree workspaceId={workspaceId} projectId={projectId ?? null} selectedId={drafting || selected === 'new' ? null : selected} onSelect={select} onDraft={startDraft} />
      </aside>
      <div className="doc-split-main">{content}</div>
      {dialog}
    </div>
  )
}
