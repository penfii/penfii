import { useEffect, useMemo, useState, type KeyboardEvent } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { DocNode } from '@penfii/contracts'
import { Button, Dialog, Select } from '../../design/ui'
import { queries } from '../../queries'
import { NewDocDialog } from './NewDocDialog'
import './knowledge.css'
import type { ShareScope } from './ShareScopePicker'
import { useI18n } from '../../i18n/I18nProvider'
import { ancestorIds, buildTree, canMoveUnder, expandAll, findInForest, flatten, forestHas, groupByProject, matchingDocs } from './treeModel'
import { ruleCode, useDocTree, useDocTreeCommands } from './useDocs'
import { CloseIcon } from '../../design/icons'

function readCollapsedGroups(workspaceId: string): Set<string> {
  try {
    const raw = localStorage.getItem(`penfii.docGroups.collapsed.${workspaceId}`)
    const parsed: unknown = raw ? JSON.parse(raw) : []
    return new Set(Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === 'string') : [])
  } catch {
    return new Set()
  }
}

/**
 * The page tree. Keyboard: arrows move and open/close, Enter opens a page, F2 renames, Delete asks to
 * delete. Dragging a page onto another moves it under that page; the same move is available from the
 * row's menu for people who do not drag. A move that would put a page under itself is refused by the
 * server and said so here.
 */
export function DocTree({
  workspaceId,
  projectId,
  selectedId,
  onSelect,
  onDraft,
  onGone,
}: {
  workspaceId: string
  projectId?: string | null
  selectedId: string | null
  /** `edit` asks the page to open in the editor (a page that was just created). */
  onSelect: (id: string | null, options?: { edit?: boolean }) => void
  /** Opens an unsaved draft instead of asking for a title up front. The parent is where it will be filed. */
  onDraft?: (parentId: string | null) => void
  /** Called after a page the person is looking at was deleted. */
  onGone?: () => void
}) {
  const { t } = useI18n()
  const tree = useDocTree(workspaceId, projectId)
  const commands = useDocTreeCommands(workspaceId)
  const nodes = useMemo<DocNode[]>(() => tree.data ?? [], [tree.data])
  const [query, setQuery] = useState('')
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => readCollapsedGroups(workspaceId))
  const shown = useMemo(() => matchingDocs(nodes, query), [nodes, query])
  const roots = useMemo(() => buildTree(shown), [shown])
  const space = useQuery({ ...queries.space(workspaceId), enabled: !projectId })
  const searching = query.trim().length > 0
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameText, setRenameText] = useState('')
  const [confirm, setConfirm] = useState<DocNode | null>(null)
  const [moving, setMoving] = useState<DocNode | null>(null)
  const [moveTarget, setMoveTarget] = useState<string>('')
  const [dropOn, setDropOn] = useState<string | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const [focused, setFocused] = useState<string | null>(null)
  // A new page asks for its title first: `parent` is where it will go.
  const [creating, setCreating] = useState<{ parentId: string | null } | null>(null)

  // The path to the selected page is always open.
  useEffect(() => {
    if (!selectedId) return
    const path = ancestorIds(nodes, selectedId)
    if (path.length === 0) return
    setExpanded((current) => {
      if (path.every((id) => current.has(id))) return current
      return new Set([...current, ...path])
    })
  }, [selectedId, nodes])

  const groups = useMemo(() => {
    if (projectId) return null
    const projects = space.data?.projects ?? []
    const order = [
      ...projects.filter((project) => project.participating !== false).map((project) => project.id),
      ...projects.filter((project) => project.participating === false).map((project) => project.id),
    ]
    return groupByProject(roots, order)
  }, [projectId, roots, space.data])
  const rows = useMemo(() => {
    if (!groups) return flatten(roots, expanded)
    const openIds = searching ? expandAll(roots) : expanded
    return groups.flatMap((group) => {
      const closed = !searching && collapsedGroups.has(group.key)
      if (!closed) return flatten(group.roots, openIds)
      if (!selectedId || !forestHas(group.roots, selectedId)) return []
      const node = findInForest(group.roots, selectedId)
      if (!node) return []
      return [{ node, depth: 0, hasChildren: node.children.length > 0, expanded: false }]
    })
  }, [collapsedGroups, expanded, groups, roots, searching, selectedId])
  useEffect(() => {
    localStorage.setItem(`penfii.docGroups.collapsed.${workspaceId}`, JSON.stringify([...collapsedGroups]))
  }, [collapsedGroups, workspaceId])

  function explain(error: unknown) {
    const code = ruleCode(error)
    if (code === 'doc_cycle') setMessage(t('docs.tree.cycle'))
    else if (code === 'doc_parent_invalid') setMessage(t('docs.tree.parentInvalid'))
    else setMessage(t('docs.tree.error'))
  }

  async function addPage(title: string, scope: ShareScope) {
    if (!creating) return
    const { parentId } = creating
    setMessage(null)
    try {
      const page = await commands.create.mutateAsync({ title, parentId, projectId: projectId ?? null, visibility: scope.visibility, scopeProjectId: scope.projectId, scopeTeamId: scope.teamId })
      if (parentId) setExpanded((current) => new Set(current).add(parentId))
      setCreating(null)
      onSelect(page.id, { edit: true })
    } catch (error) {
      if (ruleCode(error)) {
        setCreating(null)
        explain(error)
      } else throw error
    }
  }

  async function moveTo(id: string, parentId: string | null) {
    setMessage(null)
    try {
      await commands.move.mutateAsync({ id, parentId })
      if (parentId) setExpanded((current) => new Set(current).add(parentId))
    } catch (error) {
      explain(error)
    }
  }

  async function commitRename(id: string) {
    const title = renameText.trim()
    setRenaming(null)
    const current = nodes.find((n) => n.id === id)
    // A page always has a title: an emptied name is not sent.
    if (!current || !title || title === current.title) return
    try {
      await commands.rename.mutateAsync({ id, title })
    } catch (error) {
      explain(error)
    }
  }

  async function remove(node: DocNode) {
    setConfirm(null)
    try {
      await commands.remove.mutateAsync(node.id)
      if (selectedId === node.id || ancestorIds(nodes, selectedId ?? '').includes(node.id)) {
        onSelect(null)
        onGone?.()
      }
    } catch (error) {
      explain(error)
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLUListElement>) {
    if (renaming) return
    const index = rows.findIndex((r) => r.node.id === (focused ?? selectedId))
    const row = rows[index] ?? rows[0]
    if (!row) return
    const focusAt = (i: number) => {
      const target = rows[Math.max(0, Math.min(rows.length - 1, i))]
      if (target) {
        setFocused(target.node.id)
        document.getElementById(`doc-row-${target.node.id}`)?.focus()
      }
    }
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        focusAt(index + 1)
        break
      case 'ArrowUp':
        event.preventDefault()
        focusAt(index - 1)
        break
      case 'ArrowRight':
        if (row.hasChildren && !row.expanded) {
          event.preventDefault()
          setExpanded((c) => new Set(c).add(row.node.id))
        }
        break
      case 'ArrowLeft':
        if (row.expanded) {
          event.preventDefault()
          setExpanded((c) => {
            const next = new Set(c)
            next.delete(row.node.id)
            return next
          })
        }
        break
      case 'F2':
        event.preventDefault()
        if (!row.node.rights.edit) break
        setRenaming(row.node.id)
        setRenameText(row.node.title)
        break
      case 'Delete':
        event.preventDefault()
        if (row.node.rights.manage) setConfirm(row.node)
        break
      default:
    }
  }

  if (tree.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (tree.isError) return <p className="board-error" role="alert">{t('docs.tree.loadError')}</p>

  return (
    <div className="doc-tree-wrap">
      <button type="button" className="nav-add" onClick={() => (onDraft ? onDraft(null) : setCreating({ parentId: null }))}>+ {t('docs.new')}</button>
      <input className="doc-tree-search" value={query} aria-label={t('docs.tree.search')} placeholder={t('docs.tree.search')} onChange={(event) => setQuery(event.target.value)} />
      {message ? <p className="board-error" role="alert">{message}</p> : null}
      {rows.length === 0 ? <p className="tree-empty">{t('docs.tree.empty')}</p> : null}
      <ul className="doc-tree" role="tree" aria-label={t('nav.docs')} onKeyDown={onKeyDown}>
        {(groups ?? [null]).flatMap((group) => {
          const groupRows = group ? rows.filter((row) => forestHas(group.roots, row.node.id)) : rows
          const project = group?.projectId ? space.data?.projects.find((item) => item.id === group.projectId) : undefined
          const name = !group
            ? ''
            : group.projectId
              ? (project?.name ?? group.projectId)
              : t('docs.group.none')
          const openGroup = !group || searching || !collapsedGroups.has(group.key)
          const head = group ? (
            <li key={`group-${group.key}`} role="none">
              <button
                type="button"
                className="doc-group"
                aria-expanded={openGroup}
                onClick={() => setCollapsedGroups((current) => {
                  const next = new Set(current)
                  if (next.has(group.key)) next.delete(group.key)
                  else next.add(group.key)
                  return next
                })}
              >
                <span aria-hidden="true">{openGroup ? '▾' : '▸'}</span>
                {project?.ticketPrefix ? <span className="doc-group-key">{project.ticketPrefix}</span> : null}
                {name}
                <span className="doc-group-count">{group.count}</span>
              </button>
            </li>
          ) : null
          return [head, ...groupRows.map(({ node, depth, hasChildren, expanded: open }) => {
          const selected = node.id === selectedId
          return (
            <li key={node.id} role="none" style={{ paddingLeft: Math.min(depth, 3) * 14 }}>
              <div
                className={['doc-tree-row', selected ? 'is-selected' : '', dropOn === node.id ? 'is-drop' : ''].filter(Boolean).join(' ')}
                draggable={renaming !== node.id && node.rights.edit}
                onDragStart={(event) => event.dataTransfer.setData('text/x-penfii-doc', node.id)}
                onDragOver={(event) => {
                  if (event.dataTransfer.types.includes('text/x-penfii-doc')) {
                    event.preventDefault()
                    setDropOn(node.id)
                  }
                }}
                onDragLeave={() => setDropOn((current) => (current === node.id ? null : current))}
                onDrop={(event) => {
                  event.preventDefault()
                  setDropOn(null)
                  const id = event.dataTransfer.getData('text/x-penfii-doc')
                  if (id && id !== node.id && node.rights.edit) void moveTo(id, node.id)
                }}
              >
                {hasChildren ? (
                  <button
                    type="button"
                    aria-label={open ? t('docs.tree.collapse') : t('docs.tree.expand')}
                    onClick={() =>
                      setExpanded((current) => {
                        const next = new Set(current)
                        if (next.has(node.id)) next.delete(node.id)
                        else next.add(node.id)
                        return next
                      })
                    }
                  >
                    {open ? '▾' : '▸'}
                  </button>
                ) : (
                  <span aria-hidden="true" style={{ width: 22 }} />
                )}
                {renaming === node.id ? (
                  <input
                    autoFocus
                    aria-label={t('docs.tree.rename')}
                    value={renameText}
                    onChange={(event) => setRenameText(event.target.value)}
                    onBlur={() => void commitRename(node.id)}
                    onKeyDown={(event) => {
                      if (event.nativeEvent.isComposing) return
                      if (event.key === 'Enter') void commitRename(node.id)
                      if (event.key === 'Escape') setRenaming(null)
                    }}
                  />
                ) : (
                  <button
                    id={`doc-row-${node.id}`}
                    type="button"
                    role="treeitem"
                    aria-selected={selected}
                    aria-expanded={hasChildren ? open : undefined}
                    aria-level={depth + 1}
                    tabIndex={selected || (!selectedId && depth === 0 && node === rows[0]?.node) ? 0 : -1}
                    className="doc-tree-open"
                    onFocus={() => setFocused(node.id)}
                    onClick={() => onSelect(node.id)}
                    onDoubleClick={() => {
                      if (!node.rights.edit) return
                      setRenaming(node.id)
                      setRenameText(node.title)
                    }}
                  >
                    {node.visibility === 'RESTRICTED' ? <span aria-label={t('docs.restricted')} role="img">🔒 </span> : null}
                    {node.visibility === 'PROJECT' ? <span aria-label={t('docs.scope.project')} role="img">👥 </span> : null}
                    {node.title || t('docs.untitled')}
                  </button>
                )}
                <span className="doc-tree-actions">
                  {node.rights.edit ? (
                    <>
                      <button type="button" aria-label={`${t('docs.tree.addChild')}: ${node.title || t('docs.untitled')}`} onClick={() => (onDraft ? onDraft(node.id) : setCreating({ parentId: node.id }))}>＋</button>
                      <button
                        type="button"
                        aria-label={`${t('docs.tree.move')}: ${node.title || t('docs.untitled')}`}
                        onClick={() => {
                          setMoving(node)
                          setMoveTarget(node.parentId ?? '')
                        }}
                      >
                        ⇄
                      </button>
                    </>
                  ) : null}
                  {node.rights.manage ? (
                    <button type="button" aria-label={`${t('docs.delete')}: ${node.title || t('docs.untitled')}`} onClick={() => setConfirm(node)}><CloseIcon /></button>
                  ) : null}
                </span>
              </div>
            </li>
          )
        })]
        })}
      </ul>
      <NewDocDialog open={creating !== null} workspaceId={workspaceId} suggestedProjectId={projectId ?? null} onCreate={addPage} onClose={() => setCreating(null)} />
      <Dialog open={confirm !== null} title={t('docs.delete.title')} onClose={() => setConfirm(null)}>
        <p>{t('docs.delete.body', { title: confirm?.title || t('docs.untitled') })}</p>
        <div className="dialog-actions">
          <Button variant="ghost" onClick={() => setConfirm(null)}>{t('docs.cancel')}</Button>
          <Button onClick={() => confirm && void remove(confirm)}>{t('docs.delete')}</Button>
        </div>
      </Dialog>
      <Dialog open={moving !== null} title={t('docs.tree.move')} onClose={() => setMoving(null)} presentation="full">
        <label className="field">
          <span>{t('docs.tree.moveUnder')}</span>
          <Select value={moveTarget} onChange={(event) => setMoveTarget(event.target.value)}>
            <option value="">{t('docs.tree.top')}</option>
            {nodes
              .filter((n) => moving && n.id !== moving.id && canMoveUnder(nodes, moving.id, n.id))
              .map((n) => (
                <option key={n.id} value={n.id}>{n.title || t('docs.untitled')}</option>
              ))}
          </Select>
        </label>
        <div className="dialog-actions">
          <Button variant="ghost" onClick={() => setMoving(null)}>{t('docs.cancel')}</Button>
          <Button
            onClick={() => {
              const target = moving
              setMoving(null)
              if (target) void moveTo(target.id, moveTarget || null)
            }}
          >
            {t('docs.tree.move')}
          </Button>
        </div>
      </Dialog>
    </div>
  )
}
