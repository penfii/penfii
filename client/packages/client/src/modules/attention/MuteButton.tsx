import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '../../api'
import { useI18n } from '../../i18n/I18nProvider'
import { queryKeys } from '../../queryKeys'
import { queries } from '../../queries'

/** Quiet a channel: ordinary updates stop, messages that mention you still arrive. */
export function MuteButton({ channelId }: { channelId: string }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const muted = useQuery(queries.mutedChannels())
  const on = muted.data?.channelIds.includes(channelId) ?? false
  const toggle = useMutation({
    mutationFn: () => api.muteChannel(channelId, !on),
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.mutedChannels }),
  })
  return (
    <button type="button" className={on ? 'btn btn-ghost saved-on mute-button' : 'btn btn-ghost mute-button'} aria-pressed={on} title={t('mute.help')} disabled={toggle.isPending || muted.isPending} onClick={() => toggle.mutate()}>
      <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M6 9a6 6 0 0 1 12 0c0 5 2 6 2 6H4s2-1 2-6zM10 19a2 2 0 0 0 4 0" />{on ? <path d="M4 4l16 16" /> : null}</svg>
      <span className="visually-hidden">{on ? t('mute.on') : t('mute.off')}</span>
    </button>
  )
}
