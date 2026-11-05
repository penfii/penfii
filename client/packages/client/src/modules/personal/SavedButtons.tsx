import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { SavedKind } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { BookmarkIcon, PinIcon, StarIcon } from '../../design/icons'

/**
 * A star for a personal shortcut. Saving a Favorite is private, and it neither watches nor pins
 * anything: it only puts a shortcut in the person's own list.
 */
export function FavoriteButton({ workspaceId, kind, targetId, label }: { workspaceId: string; kind: SavedKind; targetId: string; label?: string }) {
  const { t } = useI18n()
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
  const text = on ? t('saved.favorite.remove') : t('saved.favorite.add')
  return (
    <button type="button" className={on ? 'icon-button saved-on' : 'icon-button'} aria-pressed={on} aria-label={label ? `${text}: ${label}` : text} title={text} disabled={toggle.isPending || favorites.isPending} onClick={() => toggle.mutate()}>
      <StarIcon />
    </button>
  )
}

/** Later: come back to this Ticket. Finishing the card never finishes the Ticket. */
export function LaterButton({ workspaceId, ticketId }: { workspaceId: string; ticketId: string }) {
  const { t } = useI18n()
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
  const text = card ? t('saved.later.remove') : t('saved.later.add')
  return (
    <button type="button" className={card ? 'btn btn-ghost saved-on' : 'btn btn-ghost'} aria-pressed={Boolean(card)} aria-label={text} title={text} disabled={toggle.isPending || later.isPending} onClick={() => toggle.mutate()}>
      <BookmarkIcon /> <span className="btn-label">{t('saved.later.short')}</span>
    </button>
  )
}

/** A Pin is shared with the whole Project and kept apart from Favorites; only a Project admin pins. */
export function PinButton({ projectId, ticketId }: { projectId: string; ticketId: string }) {
  const { t } = useI18n()
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
  if (pins.data && !pins.data.canManage) return null
  const text = pin ? t('saved.pin.remove') : t('saved.pin.add')
  return (
    <button type="button" className={pin ? 'btn btn-ghost saved-on' : 'btn btn-ghost'} aria-pressed={Boolean(pin)} aria-label={text} title={t('saved.pin.help')} disabled={toggle.isPending || pins.isPending} onClick={() => toggle.mutate()}>
      <PinIcon /> <span className="btn-label">{t('saved.pin.short')}</span>
    </button>
  )
}

export function SavedButtons({ workspaceId, ticketId, projectId }: { workspaceId: string; ticketId: string; projectId: string }) {
  return (
    <>
      <FavoriteButton workspaceId={workspaceId} kind="TICKET" targetId={ticketId} />
      <LaterButton workspaceId={workspaceId} ticketId={ticketId} />
      <PinButton projectId={projectId} ticketId={ticketId} />
    </>
  )
}
