import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { WatchTargetKind } from '@penfii/contracts'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'
import { EyeIcon } from '../../design/icons'

/**
 * Follow a Ticket, a Project or a Thread. Watching is the person's own choice; asking to stop is
 * remembered and later activity never turns it back on. Favorite and Later are separate things.
 */
export function WatchButton({ kind, targetId, className = 'btn btn-ghost' }: { kind: WatchTargetKind; targetId: string; className?: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const status = useQuery({ ...queries.subscription(kind, targetId), enabled: targetId.length > 0 })
  const on = status.data?.watching ?? false
  const toggle = useMutation({
    mutationFn: async () => {
      if (on) await api.unwatch(kind, targetId)
      else await api.watch(kind, targetId)
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: queryKeys.subscription(kind, targetId) })
      await queryClient.invalidateQueries({ queryKey: queryKeys.all.scopeDataset })
    },
  })
  const text = on ? t('watch.remove') : t('watch.add')
  return (
    <button type="button" className={on ? `${className} saved-on` : className} aria-pressed={on} aria-label={text} title={t('watch.help')} disabled={toggle.isPending || status.isPending} onClick={() => toggle.mutate()}>
      <EyeIcon /> <span className="btn-label">{on ? t('watch.on') : t('watch.off')}</span>
    </button>
  )
}
