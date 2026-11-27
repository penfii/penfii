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
import type { FavoriteFolder, FavoriteItem } from '@penfii/contracts'
import { CloseIcon } from '../../design/icons'
import { isArchivedTeam } from '../spaces/teamState'
import { notify } from '../../design/notices'
import { useFavorite } from './SavedButtons'

/** Where a saved shortcut leads. A kind this build does not know leads nowhere (no guessed link). */
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
    case 'DM':
      return `${base}/c/${ref.targetId}`
    case 'DOC':
      return `${base}/docs?doc=${ref.targetId}`
    case 'MESSAGE':
    case 'FILE':
      if (!ref.conversationId) return null
      return ref.threadRootId ? `${base}/c/${ref.conversationId}/t/${ref.threadRootId}` : `${base}/c/${ref.conversationId}`
    case 'VIEW':
      if (ref.viewTarget === 'TEAM' && ref.teamId) return `${base}/teams/${ref.teamId}?v=${ref.targetId}`
      if (ref.viewTarget === 'WORKSPACE') return `${base}/my-work?v=${ref.targetId}`
      return ref.projectId ? `${base}/projects/${ref.projectId}/work?v=${ref.targetId}` : null
    default:
      return null
  }
}

/** The name a saved shortcut is shown by. A DM with nobody else in it has no name of its own. */
export function savedTitle(ref: SavedRef, dmFallback: string): string {
  if (ref.kind === 'DM') return ref.title || dmFallback
  return ref.title ?? ''
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

export function Favorites({ workspaceId }: { workspaceId: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const favorites = useQuery(queries.favorites(workspaceId))
  const space = useQuery(queries.space(workspaceId))
  const archivedTeamIds = new Set((space.data?.teams ?? []).filter(isArchivedTeam).map((team) => team.id))
  const folders = useQuery(queries.favoriteFolders(workspaceId))
  const [name, setName] = useState('')
  const [notice, setNotice] = useState('')
  if (favorites.isPending || folders.isPending) return <p className="hint" role="status">{t('saved.favorites.loading')}</p>
  if (favorites.isError || folders.isError) {
    return (
      <div className="board-error" role="alert">
        <p>{t('saved.favorites.loadError')}</p>
        <Button type="button" variant="secondary" onClick={() => { void favorites.refetch(); void folders.refetch() }}>{t('saved.favorites.retry')}</Button>
      </div>
    )
  }
  const items = favorites.data
  const folderList = folders.data

  async function refresh() {
    await queryClient.invalidateQueries({ queryKey: queryKeys.favorites(workspaceId) })
    await queryClient.invalidateQueries({ queryKey: queryKeys.favoriteFolders(workspaceId) })
  }
  /** Something failed: say so where it cannot be missed, and show what the server has. */
  async function failed(message: string) {
    notify({ kind: 'error', message })
    await refresh()
  }
  async function move(index: number, step: -1 | 1, group: FavoriteItem[]) {
    const next = [...group]
    const [item] = next.splice(index, 1)
    if (!item) return
    next.splice(index + step, 0, item)
    // Only this group's shortcuts change places, among the places they hold: the other folders, the
    // top level and the shortcuts that cannot be opened now keep theirs.
    try {
      const saved = await api.orderFavorites(workspaceId, next.map((entry) => ({ kind: entry.kind, targetId: entry.targetId })), 'LISTED')
      queryClient.setQueryData(queryKeys.favorites(workspaceId), saved)
    } catch {
      await failed(t('saved.order.error'))
    }
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
    setNotice('')
    try {
      await api.renameFavoriteFolder(folder.id, next)
    } catch (error) {
      setNotice(isApiError(error) && error.code === 'folder_name_taken' ? t('saved.folders.taken') : t('saved.folders.badName'))
    }
    await refresh()
  }
  async function deleteFolder(folder: FavoriteFolder) {
    try {
      const result = await api.deleteFavoriteFolder(folder.id)
      setNotice(t('saved.folders.deleteDone', { count: String(result.movedToTopLevel) }))
      await refresh()
    } catch {
      await failed(t('saved.folders.deleteError'))
    }
  }
  async function moveTo(item: FavoriteItem, folderId: string) {
    try {
      await api.moveFavorite(workspaceId, item.kind, item.targetId, folderId === '' ? null : folderId)
      await refresh()
    } catch {
      await failed(t('saved.folders.moveError'))
    }
  }

  const row = (item: FavoriteItem, index: number, group: FavoriteItem[]) => (
    <FavoriteRow
      key={`${item.kind}:${item.targetId}`}
      workspaceId={workspaceId}
      item={item}
      first={index === 0}
      last={index === group.length - 1}
      archived={item.kind === 'TEAM' && archivedTeamIds.has(item.targetId)}
      folders={folderList}
      onMove={(step) => void move(index, step, group)}
      onFolder={(folderId) => void moveTo(item, folderId)}
    />
  )
  const top = items.filter((item) => !item.folderId || !folderList.some((folder) => folder.id === item.folderId))
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
              {inside.map((item, index) => row(item, index, inside))}
            </ul>
          </section>
        )
      })}
      {top.length > 0 ? (
        <ul className="result-list saved-list">
          {top.map((item, index) => row(item, index, top))}
        </ul>
      ) : null}
    </div>
  )
}

/** One shortcut on Saved. Removing it goes through the same change as every star (with Undo). */
function FavoriteRow({ workspaceId, item, first, last, archived, folders, onMove, onFolder }: {
  workspaceId: string
  item: FavoriteItem
  first: boolean
  last: boolean
  archived: boolean
  folders: FavoriteFolder[]
  onMove: (step: -1 | 1) => void
  onFolder: (folderId: string) => void
}) {
  const { t } = useI18n()
  const favorite = useFavorite(workspaceId, item.kind, item.targetId)
  const link = savedLink(workspaceId, item)
  const title = savedTitle(item, t('dm.short'))
  return (
    <li className="result-hit saved-row">
      {link ? (
        <Link to={link} className="saved-open">
          <small className="where-chip">{t(`saved.kind.${item.kind}`)}</small> <strong>{title}</strong>
          {archived ? <small className="hint"> · {t('teams.archivedTag')}</small> : null}
          {item.subtitle ? <small className="hint"> · {item.subtitle}</small> : null}
        </Link>
      ) : (
        <strong>{title}</strong>
      )}
      <span className="saved-actions">
        <select aria-label={t('saved.folders.moveTo', { title })} value={item.folderId ?? ''} onChange={(event) => onFolder(event.target.value)}>
          <option value="">{t('saved.folders.top')}</option>
          {folders.map((folder) => <option key={folder.id} value={folder.id}>{folder.name}</option>)}
        </select>
        <button type="button" className="icon-button" aria-label={t('saved.moveUp', { title })} disabled={first} onClick={() => onMove(-1)}>↑</button>
        <button type="button" className="icon-button" aria-label={t('saved.moveDown', { title })} disabled={last} onClick={() => onMove(1)}>↓</button>
        <button type="button" className="icon-button" aria-label={t('saved.favorite.removeTitle', { title })} disabled={favorite.pending} onClick={() => favorite.toggle()}><CloseIcon /></button>
      </span>
    </li>
  )
}

export function Later({ workspaceId }: { workspaceId: string }) {
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
