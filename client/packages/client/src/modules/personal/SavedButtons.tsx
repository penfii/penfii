import { useIsMutating, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { FavoriteItem, SavedKind } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { BookmarkIcon, PinIcon, StarIcon } from '../../design/icons'
import { Sheet, SheetRow } from '../../design/Sheet'
import { announceToggle, notify, type ToggleMessages } from '../../design/notices'
import { useIsMobile } from '../../design/useIsMobile'
import { useState } from 'react'

function useMessages(name: 'favorite' | 'later' | 'pin'): ToggleMessages {
  const { t } = useI18n()
  return { on: t(`notice.${name}.on`), off: t(`notice.${name}.off`), error: t('notice.save.error'), undo: t('notice.undo') }
}

/** The Favorite state of one target: not known yet, not known because the list failed to load, or known. */
export type FavoriteStatus = 'loading' | 'error' | 'on' | 'off'

const favoriteMutationKey = (workspaceId: string, kind: SavedKind, targetId: string) => ['favorite-toggle', workspaceId, kind, targetId] as const
const favoriteKey = (entry: { kind: string; targetId: string }) => `${entry.kind}:${entry.targetId}`

/**
 * The one way to add or remove a Favorite. Every star, menu row and Saved row of the same target reads
 * the same Workspace list (one request for a whole list of rows) and shares one change in progress, so
 * two controls for one target never disagree or send two changes at once. The cache changes only after
 * the server said yes, and only for that target; a refetch then confirms the whole list. A removal can be
 * undone, back into the folder and the place it was in.
 */
export function useFavorite(workspaceId: string, kind: SavedKind, targetId: string) {
  const queryClient = useQueryClient()
  const messages = useMessages('favorite')
  const favorites = useQuery({ ...queries.favorites(workspaceId), enabled: workspaceId.length > 0 })
  const mutationKey = favoriteMutationKey(workspaceId, kind, targetId)
  const busy = useIsMutating({ mutationKey }) > 0
  const item = favorites.data?.find((entry) => entry.kind === kind && entry.targetId === targetId)
  // With a list from before, a failed refetch keeps the last known state.
  const status: FavoriteStatus = favorites.data ? (item ? 'on' : 'off') : favorites.isError ? 'error' : 'loading'
  const same = (entry: FavoriteItem) => entry.kind === kind && entry.targetId === targetId
  const set = useMutation({
    mutationKey,
    mutationFn: async ({ next, folderId, before }: { next: boolean; folderId?: string | null; before?: Set<string> }): Promise<FavoriteItem | null> => {
      if (!next) {
        await api.removeFavorite(workspaceId, kind, targetId)
        return null
      }
      let saved = await api.addFavorite(workspaceId, kind, targetId)
      // Undo puts it back where it was: into its folder (if the folder is still there), and in front of the
      // shortcuts that came after it. Those keep their current order among themselves (`LISTED` names only
      // them), so a change made to the others since the removal is not undone with it.
      if (folderId && saved.folderId !== folderId) {
        try {
          await api.moveFavorite(workspaceId, kind, targetId, folderId)
          saved = { ...saved, folderId }
        } catch {
          // The folder is gone: it stays at the top level.
        }
      }
      const after = before ? (queryClient.getQueryData<FavoriteItem[]>(queryKeys.favorites(workspaceId)) ?? []).filter((entry) => before.has(favoriteKey(entry))) : []
      if (after.length > 0) {
        try {
          await api.orderFavorites(workspaceId, [saved, ...after].map((entry) => ({ kind: entry.kind, targetId: entry.targetId })), 'LISTED')
        } catch {
          // Saved again, only at the end; the refetch shows where.
        }
      }
      return saved
    },
    onSuccess: (saved) => {
      queryClient.setQueryData<FavoriteItem[]>(queryKeys.favorites(workspaceId), (list) => {
        if (!list) return list
        if (!saved) return list.filter((entry) => !same(entry))
        return list.some(same) ? list.map((entry) => (same(entry) ? saved : entry)) : [...list, saved]
      })
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.favorites(workspaceId) }),
  })
  async function change(next: boolean, folderId?: string | null, before?: Set<string>) {
    try {
      await set.mutateAsync({ next, folderId, before })
    } catch {
      notify({ kind: 'error', message: messages.error })
      return
    }
    if (next) notify({ message: messages.on })
    else notify({ message: messages.off, action: { label: messages.undo, run: () => void change(true, folderId, before) } })
  }
  function toggle() {
    if (busy) return
    if (status === 'error' || status === 'loading') {
      // Not known yet: ask again rather than guess.
      void favorites.refetch()
      return
    }
    // A removal remembers its folder and the shortcuts that came after it, for Undo.
    if (status === 'on') {
      const list = favorites.data ?? []
      const at = list.findIndex(same)
      void change(false, item?.folderId ?? null, new Set(list.slice(at + 1).map(favoriteKey)))
    } else void change(true)
  }
  return { on: status === 'on', status, pending: busy || status === 'loading', toggle }
}

export function useLater(workspaceId: string, ticketId: string) {
  const queryClient = useQueryClient()
  const messages = useMessages('later')
  const later = useQuery({ ...queries.later(workspaceId, 'ACTIVE'), enabled: workspaceId.length > 0 })
  const card = later.data?.find((item) => item.kind === 'TICKET' && item.targetId === ticketId)
  const set = useMutation({
    mutationFn: async (next: boolean) => {
      if (next) await api.addLater(workspaceId, ticketId)
      else if (card) await api.removeLater(card.id)
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.all.later }),
  })
  return { on: Boolean(card), pending: set.isPending || later.isPending, toggle: () => void announceToggle(!card, set.mutateAsync, messages) }
}

export function usePin(projectId: string, ticketId: string) {
  const queryClient = useQueryClient()
  const messages = useMessages('pin')
  const pins = useQuery(queries.projectPins(projectId))
  const pin = pins.data?.pins.find((item) => item.ticket?.id === ticketId)
  const set = useMutation({
    mutationFn: async (next: boolean) => {
      if (next) await api.addProjectPin(projectId, { ticketId })
      else if (pin) await api.removePin(pin.id)
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.projectPins(projectId) }),
  })
  // The server says who may pin: a Project admin or an editor the admin designated.
  return { on: Boolean(pin), pending: set.isPending || pins.isPending, allowed: !pins.data || pins.data.canManage, toggle: () => void announceToggle(!pin, set.mutateAsync, messages) }
}

/** What a Favorite control says: what a press will do, or that the state is not known yet. */
export function useFavoriteLabel(status: FavoriteStatus, title?: string): string {
  const { t } = useI18n()
  if (status === 'loading') return t('saved.favorite.loading')
  if (status === 'error') return t('saved.favorite.retry')
  if (title) return t(status === 'on' ? 'saved.favorite.removeTitle' : 'saved.favorite.addTitle', { title })
  return t(status === 'on' ? 'saved.favorite.remove' : 'saved.favorite.add')
}

/**
 * A star for a personal shortcut. Saving a Favorite is private, and it neither watches nor pins
 * anything: it only puts a shortcut in the person's own list. Filled while it is a Favorite. In a
 * row that opens, drags or selects, the star is its own control: a press on it does nothing else.
 */
export function FavoriteButton({ workspaceId, kind, targetId, label, className = 'icon-button' }: { workspaceId: string; kind: SavedKind; targetId: string; label?: string; className?: string }) {
  const { on, status, toggle, pending } = useFavorite(workspaceId, kind, targetId)
  const text = useFavoriteLabel(status, label)
  return (
    <button
      type="button"
      className={[className, 'favorite-star', on ? 'saved-on' : ''].filter(Boolean).join(' ')}
      aria-pressed={status === 'on' || status === 'off' ? on : undefined}
      aria-busy={pending || undefined}
      aria-label={text}
      title={text}
      disabled={pending}
      draggable={false}
      onPointerDown={(event) => event.stopPropagation()}
      onDragStart={(event) => { event.preventDefault(); event.stopPropagation() }}
      onClick={(event) => { event.preventDefault(); event.stopPropagation(); toggle() }}
    >
      <StarIcon filled={on} />
    </button>
  )
}

/** A Favorite row for a menu (`role="menu"`): a checkbox item that says what it does. */
export function FavoriteMenuItem({ workspaceId, kind, targetId, onDone }: { workspaceId: string; kind: SavedKind; targetId: string; onDone?: () => void }) {
  const { on, status, toggle, pending } = useFavorite(workspaceId, kind, targetId)
  const text = useFavoriteLabel(status)
  return (
    <button type="button" role="menuitemcheckbox" aria-checked={status === 'on' || status === 'off' ? on : 'mixed'} disabled={pending} onClick={() => { toggle(); onDone?.() }}>
      <StarIcon filled={on} /> {text}
    </button>
  )
}

/** Later: come back to this Ticket. Finishing the card never finishes the Ticket. */
export function LaterButton({ workspaceId, ticketId }: { workspaceId: string; ticketId: string }) {
  const { t } = useI18n()
  const { on, toggle, pending } = useLater(workspaceId, ticketId)
  const text = on ? t('saved.later.remove') : t('saved.later.add')
  return (
    <button type="button" className={on ? 'btn btn-ghost saved-on' : 'btn btn-ghost'} aria-pressed={on} aria-label={text} title={text} disabled={pending} onClick={toggle}>
      <BookmarkIcon /> <span className="btn-label">{t('saved.later.short')}</span>
    </button>
  )
}

/** A Pin is shared with the whole Project and kept apart from Favorites; only a Project admin pins. */
export function PinButton({ projectId, ticketId }: { projectId: string; ticketId: string }) {
  const { t } = useI18n()
  const { on, toggle, pending, allowed } = usePin(projectId, ticketId)
  if (!allowed) return null
  const text = on ? t('saved.pin.remove') : t('saved.pin.add')
  return (
    <button type="button" className={on ? 'btn btn-ghost saved-on' : 'btn btn-ghost'} aria-pressed={on} aria-label={text} title={t('saved.pin.help')} disabled={pending} onClick={toggle}>
      <PinIcon /> <span className="btn-label">{t('saved.pin.short')}</span>
    </button>
  )
}

export function SavedButtons({ workspaceId, ticketId, projectId }: { workspaceId: string; ticketId: string; projectId: string }) {
  const mobile = useIsMobile()
  if (mobile) return <SaveSheet workspaceId={workspaceId} ticketId={ticketId} projectId={projectId} />
  return (
    <>
      <FavoriteButton workspaceId={workspaceId} kind="TICKET" targetId={ticketId} />
      <LaterButton workspaceId={workspaceId} ticketId={ticketId} />
      <PinButton projectId={projectId} ticketId={ticketId} />
    </>
  )
}

/** On a phone the three saves share one button and a sheet: a Ticket goes to Favorites or Later, and an admin may also pin it. */
function SaveSheet({ workspaceId, ticketId, projectId }: { workspaceId: string; ticketId: string; projectId: string }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  const favorite = useFavorite(workspaceId, 'TICKET', ticketId)
  const favoriteText = useFavoriteLabel(favorite.status)
  const later = useLater(workspaceId, ticketId)
  const pin = usePin(projectId, ticketId)
  const saved = favorite.on || later.on || pin.on
  return (
    <>
      <button type="button" className={saved ? 'icon-button saved-on' : 'icon-button'} aria-haspopup="dialog" aria-label={t('mobile.save')} title={t('mobile.save')} onClick={() => setOpen(true)}>
        <BookmarkIcon />
      </button>
      <Sheet open={open} title={t('mobile.save')} onClose={() => setOpen(false)}>
        <SheetRow disabled={favorite.pending} selected={favorite.on} hint={t('saved.tab.favorites')} onClick={() => favorite.toggle()}>{favoriteText}</SheetRow>
        <SheetRow disabled={later.pending} selected={later.on} hint={t('saved.tab.later')} onClick={() => later.toggle()}>{later.on ? t('saved.later.remove') : t('saved.later.add')}</SheetRow>
        {pin.allowed ? <SheetRow disabled={pin.pending} selected={pin.on} hint={t('saved.pin.help')} onClick={() => pin.toggle()}>{pin.on ? t('saved.pin.remove') : t('saved.pin.add')}</SheetRow> : null}
      </Sheet>
    </>
  )
}
