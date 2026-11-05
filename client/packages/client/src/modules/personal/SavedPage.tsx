import { useState } from 'react'
import { Link } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { LaterItem, LaterState, SavedRef } from '@penfii/contracts'
import { api } from '../../api'
import { Button, EmptyState, Tabs } from '../../design/ui'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { ReminderControl } from '../attention/ReminderControl'
import { isApiError } from '@penfii/api-client'
import type { FavoriteFolder, FavoriteItem, SavedKind } from '@penfii/contracts'
import { CloseIcon } from '../../design/icons'
import { isArchivedTeam } from '../spaces/teamState'

/** Where a saved shortcut leads. */
export function savedLink(workspaceId: string, ref: SavedRef): string | null {
  const base = `/w/${workspaceId}`
  switch (ref.kind) {
    case 'PROJECT':
      return `${base}/projects/${ref.targetId}`
    case 'TEAM':
      return `${base}/teams/${ref.targetId}`
    case 'TICKET':
      return ref.projectId ? `${base}/projects/${ref.projectId}/work?panel=ticket:${ref.targetId}` : null
    case 'CHANNEL':
      return ref.projectId ? `${base}/projects/${ref.projectId}/chat/${ref.targetId}` : `${base}/c/${ref.targetId}`
    case 'MESSAGE':
    case 'FILE':
      if (!ref.conversationId) return null
      return ref.threadRootId ? `${base}/c/${ref.conversationId}/t/${ref.threadRootId}` : `${base}/c/${ref.conversationId}`
    default:
      if (ref.viewTarget === 'TEAM' && ref.teamId) return `${base}/teams/${ref.teamId}?v=${ref.targetId}`
      if (ref.viewTarget === 'WORKSPACE') return `${base}/my-work?v=${ref.targetId}`
      return ref.projectId ? `${base}/projects/${ref.projectId}/work?v=${ref.targetId}` : null
  }
}

type Mode = 'favorites' | 'later'

/**
 * Saved: the person's own shortcuts (Favorites, in a place they choose) and the things to come
 * back to (Later). Both are private. A target that can no longer be opened shows no title.
 */
export function SavedPage({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const [mode, setMode] = useState<Mode>('favorites')
  return (
    <div className="page-scroll">
      <header className="page-intro">
        <h1>{t('saved.title')}</h1>
        <p>{t('saved.lead')}</p>
      </header>
      <Tabs
        tabs={[
          { id: 'favorites', label: t('saved.tab.favorites') },
          { id: 'later', label: t('saved.tab.later') },
        ]}
        value={mode}
        onChange={(id) => setMode(id as Mode)}
      />
      <div className="project-pad">
        {mode === 'favorites' ? <Favorites workspaceId={workspaceId} /> : <Later workspaceId={workspaceId} />}
      </div>
    </div>
  )
}

function Favorites({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const favorites = useQuery(queries.favorites(workspaceId))
  const space = useQuery(queries.space(workspaceId))
  const archivedTeamIds = new Set((space.data?.teams ?? []).filter(isArchivedTeam).map((team) => team.id))
  const folders = useQuery(queries.favoriteFolders(workspaceId))
  const [name, setName] = useState('')
  const [notice, setNotice] = useState('')
  if (favorites.isPending || folders.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (favorites.isError || folders.isError) return <p className="board-error" role="alert">{t('saved.error')}</p>
  const items = favorites.data
  const folderList = folders.data

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.favorites(workspaceId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.favoriteFolders(workspaceId) })
  }
  async function move(index: number, step: -1 | 1, group: FavoriteItem[]) {
    const next = [...group]
    const [item] = next.splice(index, 1)
    if (!item) return
    next.splice(index + step, 0, item)
    // The order is one list; the named ones come first in the order given.
    await api.orderFavorites(workspaceId, next.map((entry) => ({ kind: entry.kind, targetId: entry.targetId })))
    await refresh()
  }
  async function remove(item: SavedRef) {
    await api.removeFavorite(workspaceId, item.kind as SavedKind, item.targetId)
    await refresh()
  }
  async function create() {
    setNotice('')
    try {
      await api.createFavoriteFolder(workspaceId, name)
      setName('')
    } catch (error) {
      if (isApiError(error) && error.code === 'folder_name_taken') setNotice(t('saved.folders.taken'))
      else if (isApiError(error) && error.code === 'folder_limit') setNotice(t('saved.folders.limit'))
      else setNotice(t('saved.folders.badName'))
    }
    await refresh()
  }
  async function rename(folder: FavoriteFolder) {
    const next = window.prompt(t('saved.folders.name'), folder.name)
    if (!next || next.trim() === folder.name) return
    try {
      await api.renameFavoriteFolder(folder.id, next)
    } catch {
      setNotice(t('saved.folders.taken'))
    }
    await refresh()
  }
  async function deleteFolder(folder: FavoriteFolder) {
    const result = await api.deleteFavoriteFolder(folder.id)
    setNotice(t('saved.folders.deleteDone', { count: String(result.movedToTopLevel) }))
    await refresh()
  }
  async function moveTo(item: FavoriteItem, folderId: string) {
    await api.moveFavorite(workspaceId, item.kind, item.targetId, folderId === '' ? null : folderId)
    await refresh()
  }

  function Row({ item, index, group }: { item: FavoriteItem; index: number; group: FavoriteItem[] }) {
    const link = savedLink(workspaceId, item)
    return (
      <li className="result-hit saved-row">
        {link ? (
          <Link to={link} className="saved-open">
            <small className="where-chip">{t(`saved.kind.${item.kind}`)}</small> <strong>{item.title}</strong>
            {item.kind === 'TEAM' && archivedTeamIds.has(item.targetId) ? <small className="hint"> · {t('teams.archivedTag')}</small> : null}
            {item.subtitle ? <small className="hint"> · {item.subtitle}</small> : null}
          </Link>
        ) : (
          <strong>{item.title}</strong>
        )}
        <span className="saved-actions">
          <select aria-label={t('saved.folders.moveTo', { title: item.title ?? '' })} value={item.folderId ?? ''} onChange={(event) => void moveTo(item, event.target.value)}>
            <option value="">{t('saved.folders.top')}</option>
            {folderList.map((folder) => <option key={folder.id} value={folder.id}>{folder.name}</option>)}
          </select>
          <button type="button" className="icon-button" aria-label={t('saved.moveUp', { title: item.title ?? '' })} disabled={index === 0} onClick={() => void move(index, -1, group)}>↑</button>
          <button type="button" className="icon-button" aria-label={t('saved.moveDown', { title: item.title ?? '' })} disabled={index === group.length - 1} onClick={() => void move(index, 1, group)}>↓</button>
          <button type="button" className="icon-button" aria-label={t('saved.favorite.removeTitle', { title: item.title ?? '' })} onClick={() => void remove(item)}><CloseIcon /></button>
        </span>
      </li>
    )
  }

  const top = items.filter((item) => !item.folderId)
  return (
    <div className="favorites">
      <form className="folder-create" onSubmit={(event) => { event.preventDefault(); void create() }}>
        <input value={name} maxLength={60} placeholder={t('saved.folders.name')} aria-label={t('saved.folders.name')} onChange={(event) => setName(event.target.value)} />
        <Button type="submit" variant="secondary" disabled={name.trim() === ''}>{t('saved.folders.create')}</Button>
      </form>
      {notice ? <p className="hint" role="status">{notice}</p> : null}
      {items.length === 0 && folderList.length === 0 ? <EmptyState title={t('saved.favorites.empty')} body={t('saved.favorites.emptyBody')} /> : null}
      {folderList.map((folder) => {
        const inside = items.filter((item) => item.folderId === folder.id)
        return (
          <section key={folder.id} className="saved-folder" aria-label={folder.name}>
            <h3>
              {folder.name}
              <span className="saved-actions">
                <button type="button" className="btn btn-ghost" onClick={() => void rename(folder)}>{t('saved.folders.rename')}</button>
                <button type="button" className="btn btn-ghost" title={t('saved.folders.deleteHelp')} onClick={() => void deleteFolder(folder)}>{t('saved.folders.delete')}</button>
              </span>
            </h3>
            {inside.length === 0 ? <p className="hint">{t('saved.folders.empty')}</p> : null}
            <ul className="result-list saved-list">
              {inside.map((item, index) => <Row key={`${item.kind}:${item.targetId}`} item={item} index={index} group={inside} />)}
            </ul>
          </section>
        )
      })}
      {top.length > 0 ? (
        <ul className="result-list saved-list">
          {top.map((item, index) => <Row key={`${item.kind}:${item.targetId}`} item={item} index={index} group={top} />)}
        </ul>
      ) : null}
    </div>
  )
}

function Later({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const [state, setState] = useState<LaterState>('ACTIVE')
  return (
    <div>
      <Tabs
        tabs={(['ACTIVE', 'DONE', 'ARCHIVED'] as LaterState[]).map((id) => ({ id, label: t(`saved.later.state.${id}`) }))}
        value={state}
        onChange={(id) => setState(id as LaterState)}
      />
      <LaterList workspaceId={workspaceId} state={state} />
    </div>
  )
}

function LaterList({ workspaceId, state }: { workspaceId: string; state: LaterState }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const later = useQuery(queries.later(workspaceId, state))
  if (later.isPending) return <p className="hint" role="status">{t('loading')}</p>
  if (later.isError) return <p className="board-error" role="alert">{t('saved.error')}</p>
  if (later.data.length === 0) return <EmptyState title={t('saved.later.empty')} body={t('saved.later.emptyBody')} />

  async function change(item: LaterItem, next: LaterState) {
    try {
      await api.updateLater(item.id, { expectedVersion: item.version, state: next })
    } finally {
      await queryClient.invalidateQueries({ queryKey: queryKeys.all.later })
    }
  }
  async function remove(item: LaterItem) {
    await api.removeLater(item.id)
    await queryClient.invalidateQueries({ queryKey: queryKeys.all.later })
  }
  return (
    <ul className="result-list saved-list">
      {later.data.map((item) => {
        const link = savedLink(workspaceId, item)
        return (
          <li key={item.id} className="result-hit saved-row">
            <div className="saved-open">
              {item.available && link ? (
                <Link to={link}><small className="where-chip">{t(`saved.kind.${item.kind}`)}</small> <strong>{item.title}</strong></Link>
              ) : (
                <strong>{t('saved.unavailable')}</strong>
              )}
              {item.subtitle && item.available ? <small className="hint">{item.subtitle}</small> : null}
              {item.note ? <p className="hint">{item.note}</p> : null}
              {item.available ? <ReminderControl item={item} /> : null}
            </div>
            <span className="saved-actions">
              {state === 'ACTIVE' ? <Button type="button" variant="secondary" onClick={() => void change(item, 'DONE')}>{t('saved.later.done')}</Button> : null}
              {state === 'ACTIVE' ? <Button type="button" variant="ghost" onClick={() => void change(item, 'ARCHIVED')}>{t('saved.later.archive')}</Button> : null}
              {state !== 'ACTIVE' ? <Button type="button" variant="secondary" onClick={() => void change(item, 'ACTIVE')}>{t('saved.later.reopen')}</Button> : null}
              <button type="button" className="icon-button" aria-label={t('saved.later.removeTitle')} onClick={() => void remove(item)}><CloseIcon /></button>
            </span>
          </li>
        )
      })}
    </ul>
  )
}
