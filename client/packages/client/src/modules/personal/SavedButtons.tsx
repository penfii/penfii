import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { SavedKind } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { BookmarkIcon, PinIcon, StarIcon } from '../../design/icons'
import { Sheet, SheetRow } from '../../design/Sheet'
import { useIsMobile } from '../../design/useIsMobile'
import { useState } from 'react'

function useFavorite(workspaceId: string, kind: SavedKind, targetId: string) {
  const queryClient = useQueryClient()
  const favorites = useQuery({ ...queries.favorites(workspaceId), enabled: workspaceId.length > 0 })
  const on = favorites.data?.some((item) => item.kind === kind && item.targetId === targetId) ?? false
  const toggle = useMutation({
    mutationFn: async () => {
      if (on) await api.removeFavorite(workspaceId, kind, targetId)
      else await api.addFavorite(workspaceId, kind, targetId)
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.favorites(workspaceId) }),
  })
  return { on, toggle, pending: toggle.isPending || favorites.isPending }
}

function useLater(workspaceId: string, ticketId: string) {
  const queryClient = useQueryClient()
  const later = useQuery({ ...queries.later(workspaceId, 'ACTIVE'), enabled: workspaceId.length > 0 })
  const card = later.data?.find((item) => item.kind === 'TICKET' && item.targetId === ticketId)
  const toggle = useMutation({
    mutationFn: async () => {
      if (card) await api.removeLater(card.id)
      else await api.addLater(workspaceId, ticketId)
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.all.later }),
  })
  return { on: Boolean(card), toggle, pending: toggle.isPending || later.isPending }
}

function usePin(projectId: string, ticketId: string) {
  const queryClient = useQueryClient()
  const pins = useQuery(queries.projectPins(projectId))
  const pin = pins.data?.pins.find((item) => item.ticket?.id === ticketId)
  const toggle = useMutation({
    mutationFn: async () => {
      if (pin) await api.removePin(pin.id)
      else await api.addProjectPin(projectId, { ticketId })
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.projectPins(projectId) }),
  })
  // The server says who may pin: a Project admin or an editor the admin designated.
  return { on: Boolean(pin), toggle, pending: toggle.isPending || pins.isPending, allowed: !pins.data || pins.data.canManage }
}

/**
 * A star for a personal shortcut. Saving a Favorite is private, and it neither watches nor pins
 * anything: it only puts a shortcut in the person's own list.
 */
export function FavoriteButton({ workspaceId, kind, targetId, label }: { workspaceId: string; kind: SavedKind; targetId: string; label?: string }) {
  const { t } = useI18n()
  const { on, toggle, pending } = useFavorite(workspaceId, kind, targetId)
  const text = on ? t('saved.favorite.remove') : t('saved.favorite.add')
  return (
    <button type="button" className={on ? 'icon-button saved-on' : 'icon-button'} aria-pressed={on} aria-label={label ? `${text}: ${label}` : text} title={text} disabled={pending} onClick={() => toggle.mutate()}>
      <StarIcon />
    </button>
  )
}

/** Later: come back to this Ticket. Finishing the card never finishes the Ticket. */
export function LaterButton({ workspaceId, ticketId }: { workspaceId: string; ticketId: string }) {
  const { t } = useI18n()
  const { on, toggle, pending } = useLater(workspaceId, ticketId)
  const text = on ? t('saved.later.remove') : t('saved.later.add')
  return (
    <button type="button" className={on ? 'btn btn-ghost saved-on' : 'btn btn-ghost'} aria-pressed={on} aria-label={text} title={text} disabled={pending} onClick={() => toggle.mutate()}>
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
    <button type="button" className={on ? 'btn btn-ghost saved-on' : 'btn btn-ghost'} aria-pressed={on} aria-label={text} title={t('saved.pin.help')} disabled={pending} onClick={() => toggle.mutate()}>
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
  const later = useLater(workspaceId, ticketId)
  const pin = usePin(projectId, ticketId)
  const saved = favorite.on || later.on || pin.on
  return (
    <>
      <button type="button" className={saved ? 'icon-button saved-on' : 'icon-button'} aria-haspopup="dialog" aria-label={t('mobile.save')} title={t('mobile.save')} onClick={() => setOpen(true)}>
        <BookmarkIcon />
      </button>
      <Sheet open={open} title={t('mobile.save')} onClose={() => setOpen(false)}>
        <SheetRow disabled={favorite.pending} selected={favorite.on} hint={t('saved.tab.favorites')} onClick={() => favorite.toggle.mutate()}>{favorite.on ? t('saved.favorite.remove') : t('saved.favorite.add')}</SheetRow>
        <SheetRow disabled={later.pending} selected={later.on} hint={t('saved.tab.later')} onClick={() => later.toggle.mutate()}>{later.on ? t('saved.later.remove') : t('saved.later.add')}</SheetRow>
        {pin.allowed ? <SheetRow disabled={pin.pending} selected={pin.on} hint={t('saved.pin.help')} onClick={() => pin.toggle.mutate()}>{pin.on ? t('saved.pin.remove') : t('saved.pin.add')}</SheetRow> : null}
      </Sheet>
    </>
  )
}
